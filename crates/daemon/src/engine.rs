use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use beankey_converter::{
    CompleteAction, ComposingCount as ConverterComposingCount, ConversionSession, DictionaryEntry,
    DictionaryError, DictionaryMetadata, DictionaryStore, ForeignCompletionProvider, FormatReport,
    HunspellCompleter, HunspellError, InputModifier, InputStyle as ConverterInputStyle, InputTable,
    InputTableId, InputTableRegistry, KeyboardLanguage, LearningError, LearningMemory,
    LearningMode, LmTypoConfig, NGramError, NGramLanguageModel, NormalConverter,
    PostCompositionPrediction, PostCompositionPredictor, PredictionMode, RequestOptions,
    SelectionError, TextReplacer, TextReplacerError, TypoCorrectionMode, ZenzLanguageModel,
    ZenzV3Config, ZenzVersionConfig, experimental_typo_correction,
};
use beankey_llama::LlamaError;
use serde::Deserialize;

use crate::config::{
    ConversionConfig, DaemonConfig, InputStyleConfig, KeyboardLanguageConfig, LearningConfig,
    LearningModeConfig, LmTypoLanguageModel, PredictionConfig, TypoCorrectionConfig,
};
use crate::protocol::composing_count::Count;
use crate::protocol::envelope::Payload;
use crate::protocol::protocol_error::Code;
use crate::zenz;
use crate::{LlamaModel, PROTOCOL_VERSION, protocol};

const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_KP_ENTER: u32 = 0xff8d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_LEFT: u32 = 0xff51;
const KEY_UP: u32 = 0xff52;
const KEY_RIGHT: u32 = 0xff53;
const KEY_DOWN: u32 = 0xff54;
const KEY_DELETE: u32 = 0xffff;
const KEY_SPACE: u32 = 0x20;
const SHIFT_MODIFIER: u32 = 1;

struct SessionState {
    conversion: ConversionSession,
    input_style: ConverterInputStyle,
    last_request_id: u64,
    selected_candidate: usize,
    surrounding: SurroundingContext,
    request_options: RequestOptions,
    last_committed: Option<beankey_converter::Candidate>,
    post_predictions: Vec<PostCompositionPrediction>,
    live_candidate: Option<beankey_converter::Candidate>,
    typo_corrections: Vec<beankey_converter::LmTypoCandidate>,
    lm_typo_available: bool,
    learning_available: bool,
    learning_writable: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SurroundingContext {
    left: Option<String>,
    right: Option<String>,
}

type SessionRequestResult<T> = Result<T, (Code, String)>;

pub struct Engine {
    dictionary: DictionaryStore,
    tables: InputTableRegistry,
    sessions: HashMap<String, SessionState>,
    zenz_model: Option<Box<dyn ZenzLanguageModel>>,
    zenz_version: ZenzVersionConfig,
    zenz_rich_candidates: bool,
    zenz_inference_limit: usize,
    zenz_predictive_input: bool,
    zenz_personalization: Option<zenz::ZenzPersonalizationModels>,
    lm_typo_enabled: bool,
    lm_typo_language_model: LmTypoLanguageModel,
    lm_typo_config: LmTypoConfig,
    lm_typo_ngram: Option<NGramLanguageModel>,
    foreign_completion_provider: Option<Arc<dyn ForeignCompletionProvider>>,
    learning_memory: Option<LearningMemory>,
    learning_available: bool,
    learning_writable: bool,
    text_replacer: Option<TextReplacer>,
    user_dictionary: Vec<DictionaryEntry>,
    user_dictionary_directory: Option<PathBuf>,
    request_options: RequestOptions,
    live_conversion: bool,
    default_input_style: ConverterInputStyle,
    default_keyboard_language: KeyboardLanguage,
}

#[derive(Debug)]
pub enum EngineOpenError {
    Dictionary(DictionaryError),
    Llama(LlamaError),
    Hunspell(HunspellError),
    Learning(LearningError),
    TextReplacer(TextReplacerError),
    ConversionResource(ConversionResourceError),
    NGram(NGramError),
}

impl fmt::Display for EngineOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dictionary(error) => error.fmt(formatter),
            Self::Llama(error) => error.fmt(formatter),
            Self::Hunspell(error) => error.fmt(formatter),
            Self::Learning(error) => error.fmt(formatter),
            Self::TextReplacer(error) => error.fmt(formatter),
            Self::ConversionResource(error) => error.fmt(formatter),
            Self::NGram(error) => error.fmt(formatter),
        }
    }
}

impl Error for EngineOpenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Dictionary(error) => Some(error),
            Self::Llama(error) => Some(error),
            Self::Hunspell(error) => Some(error),
            Self::Learning(error) => Some(error),
            Self::TextReplacer(error) => Some(error),
            Self::ConversionResource(error) => Some(error),
            Self::NGram(error) => Some(error),
        }
    }
}

impl From<DictionaryError> for EngineOpenError {
    fn from(value: DictionaryError) -> Self {
        Self::Dictionary(value)
    }
}

impl From<LlamaError> for EngineOpenError {
    fn from(value: LlamaError) -> Self {
        Self::Llama(value)
    }
}

impl From<HunspellError> for EngineOpenError {
    fn from(value: HunspellError) -> Self {
        Self::Hunspell(value)
    }
}

impl From<LearningError> for EngineOpenError {
    fn from(value: LearningError) -> Self {
        Self::Learning(value)
    }
}

impl From<TextReplacerError> for EngineOpenError {
    fn from(value: TextReplacerError) -> Self {
        Self::TextReplacer(value)
    }
}

impl From<ConversionResourceError> for EngineOpenError {
    fn from(value: ConversionResourceError) -> Self {
        Self::ConversionResource(value)
    }
}

impl From<NGramError> for EngineOpenError {
    fn from(value: NGramError) -> Self {
        Self::NGram(value)
    }
}

#[derive(Debug)]
pub enum ConversionResourceError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    UserDictionary(serde_json::Error),
    InvalidInputTable {
        name: String,
    },
    Dictionary(DictionaryError),
}

impl fmt::Display for ConversionResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::UserDictionary(error) => write!(formatter, "invalid user dictionary: {error}"),
            Self::InvalidInputTable { name } => write!(formatter, "invalid input table {name}"),
            Self::Dictionary(error) => error.fmt(formatter),
        }
    }
}

impl Error for ConversionResourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::UserDictionary(error) => Some(error),
            Self::Dictionary(error) => Some(error),
            Self::InvalidInputTable { .. } => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserDictionaryItem {
    word: String,
    reading: String,
    #[serde(default)]
    #[serde(rename = "hint")]
    _hint: Option<String>,
}

impl Engine {
    pub fn open(dictionary_path: impl AsRef<Path>) -> Result<Self, DictionaryError> {
        Ok(Self {
            dictionary: DictionaryStore::open(dictionary_path.as_ref().to_path_buf())?,
            tables: InputTableRegistry::new(),
            sessions: HashMap::new(),
            zenz_model: None,
            zenz_version: ZenzVersionConfig::default(),
            zenz_rich_candidates: false,
            zenz_inference_limit: zenz::DEFAULT_INFERENCE_LIMIT,
            zenz_predictive_input: false,
            zenz_personalization: None,
            lm_typo_enabled: false,
            lm_typo_language_model: LmTypoLanguageModel::Zenz,
            lm_typo_config: LmTypoConfig::default(),
            lm_typo_ngram: None,
            foreign_completion_provider: None,
            learning_memory: None,
            learning_available: false,
            learning_writable: false,
            text_replacer: None,
            user_dictionary: Vec::new(),
            user_dictionary_directory: None,
            request_options: default_request_options(),
            live_conversion: false,
            default_input_style: ConverterInputStyle::RomanToKana,
            default_keyboard_language: KeyboardLanguage::Japanese,
        })
    }

    pub fn open_with_zenz_model(
        dictionary_path: impl AsRef<Path>,
        model: Box<dyn ZenzLanguageModel>,
    ) -> Result<Self, DictionaryError> {
        let mut engine = Self::open(dictionary_path)?;
        engine.zenz_model = Some(model);
        Ok(engine)
    }

    pub fn open_with_llama(
        dictionary_path: impl AsRef<Path>,
        model_path: impl AsRef<Path>,
        backend_directory: impl AsRef<Path>,
    ) -> Result<Self, EngineOpenError> {
        Ok(Self::open_with_zenz_model(
            dictionary_path,
            Box::new(LlamaModel::load(model_path, backend_directory)?),
        )?)
    }

    pub fn open_with_config(
        config: &DaemonConfig,
        learning_directory: impl AsRef<Path>,
    ) -> Result<Self, EngineOpenError> {
        let mut engine = Self::open_with_llama(
            &config.dictionary,
            &config.model,
            &config.llama_backend_directory,
        )?;
        engine.load_hunspell(
            &config.hunspell.english_dictionary,
            &config.hunspell.greek_dictionary,
        )?;
        engine.load_learning(learning_directory, &config.learning)?;
        engine.load_text_replacer(&config.emoji_dictionary)?;
        engine.load_conversion_resources(&config.conversion)?;
        engine.apply_conversion_options(&config.conversion)?;
        engine.apply_zenz_options(&config.zenz);
        engine.load_zenz_personalization(&config.zenz)?;
        engine.load_lm_typo(&config.lm_typo)?;
        Ok(engine)
    }

    pub fn open_with_hunspell(
        dictionary_path: impl AsRef<Path>,
        english_dictionary: impl AsRef<Path>,
        greek_dictionary: impl AsRef<Path>,
    ) -> Result<Self, EngineOpenError> {
        let mut engine = Self::open(dictionary_path)?;
        engine.load_hunspell(english_dictionary, greek_dictionary)?;
        Ok(engine)
    }

    pub fn open_with_learning(
        dictionary_path: impl AsRef<Path>,
        learning_directory: impl AsRef<Path>,
    ) -> Result<Self, EngineOpenError> {
        let mut engine = Self::open(dictionary_path)?;
        engine.load_learning(learning_directory, &LearningConfig::default())?;
        Ok(engine)
    }

    pub fn open_with_emoji(
        dictionary_path: impl AsRef<Path>,
        emoji_dictionary: impl AsRef<Path>,
    ) -> Result<Self, EngineOpenError> {
        let mut engine = Self::open(dictionary_path)?;
        engine.load_text_replacer(emoji_dictionary)?;
        Ok(engine)
    }

    pub fn open_with_conversion_resources(
        dictionary_path: impl AsRef<Path>,
        conversion: &ConversionConfig,
    ) -> Result<Self, EngineOpenError> {
        let mut engine = Self::open(dictionary_path)?;
        engine.load_conversion_resources(conversion)?;
        engine.apply_conversion_options(conversion)?;
        Ok(engine)
    }

    fn load_hunspell(
        &mut self,
        english_dictionary: impl AsRef<Path>,
        greek_dictionary: impl AsRef<Path>,
    ) -> Result<(), HunspellError> {
        self.foreign_completion_provider = Some(Arc::new(HunspellCompleter::open(
            english_dictionary,
            greek_dictionary,
        )?));
        Ok(())
    }

    fn load_learning(
        &mut self,
        learning_directory: impl AsRef<Path>,
        config: &LearningConfig,
    ) -> Result<(), LearningError> {
        let mode = match config.mode {
            LearningModeConfig::InputAndOutput => LearningMode::InputAndOutput,
            LearningModeConfig::OnlyOutput => LearningMode::OnlyOutput,
            LearningModeConfig::Nothing => LearningMode::Nothing,
        };
        self.learning_memory = Some(LearningMemory::open(
            learning_directory.as_ref().to_path_buf(),
            mode,
            config.max_count,
        )?);
        self.learning_available = config.mode != LearningModeConfig::Nothing;
        self.learning_writable = config.mode == LearningModeConfig::InputAndOutput;
        Ok(())
    }

    fn load_text_replacer(
        &mut self,
        emoji_dictionary: impl AsRef<Path>,
    ) -> Result<(), TextReplacerError> {
        self.text_replacer = Some(TextReplacer::open(emoji_dictionary)?);
        Ok(())
    }

    fn load_conversion_resources(
        &mut self,
        conversion: &ConversionConfig,
    ) -> Result<(), ConversionResourceError> {
        if let Some(path) = &conversion.user_dictionary {
            let source =
                fs::read_to_string(path).map_err(|source| ConversionResourceError::Read {
                    path: path.clone(),
                    source,
                })?;
            let items: Vec<UserDictionaryItem> =
                serde_json::from_str(&source).map_err(ConversionResourceError::UserDictionary)?;
            self.user_dictionary = items
                .into_iter()
                .map(|item| DictionaryEntry {
                    word: item.word,
                    ruby: to_katakana(&item.reading),
                    left_id: 1288,
                    right_id: 1288,
                    meaning_id: 501,
                    base_value: -10.0,
                    adjustment: 0.0,
                    metadata: DictionaryMetadata::USER_DICTIONARY,
                })
                .collect();
        }
        if let Some(path) = &conversion.user_dictionary_directory {
            beankey_converter::UserDictionary::open(path.clone())
                .map_err(ConversionResourceError::Dictionary)?;
            self.user_dictionary_directory = Some(path.clone());
        }
        for (name, path) in &conversion.custom_input_tables {
            let source =
                fs::read_to_string(path).map_err(|source| ConversionResourceError::Read {
                    path: path.clone(),
                    source,
                })?;
            if name.is_empty()
                || !matches!(
                    InputTable::check_custom_tsv(&source),
                    FormatReport::FullyValid
                )
            {
                return Err(ConversionResourceError::InvalidInputTable { name: name.clone() });
            }
            self.tables
                .register(name.clone(), InputTable::from_custom_tsv(&source));
        }
        Ok(())
    }

    fn apply_conversion_options(
        &mut self,
        conversion: &ConversionConfig,
    ) -> Result<(), ConversionResourceError> {
        self.request_options = RequestOptions {
            n_best: conversion.n_best,
            japanese_prediction: prediction_mode(conversion.japanese_prediction),
            full_width_roman: conversion.full_width_roman,
            half_width_kana: conversion.half_width_kana,
            typo_correction: typo_correction_mode(conversion.typo_correction),
            foreign_prediction: prediction_mode(conversion.foreign_prediction),
            typography: conversion.typography,
            ..default_request_options()
        };
        self.live_conversion = conversion.live_conversion;
        self.default_input_style = match conversion.input_style {
            InputStyleConfig::Direct => ConverterInputStyle::Direct,
            InputStyleConfig::RomanToKana => ConverterInputStyle::RomanToKana,
            InputStyleConfig::Azik => ConverterInputStyle::Mapped(InputTableId::DefaultAzik),
            InputStyleConfig::KanaJis => ConverterInputStyle::Mapped(InputTableId::DefaultKanaJis),
            InputStyleConfig::KanaUs => ConverterInputStyle::Mapped(InputTableId::DefaultKanaUs),
            InputStyleConfig::Custom => {
                let name = conversion
                    .custom_input_table
                    .as_ref()
                    .filter(|name| self.tables.contains(name))
                    .ok_or_else(|| ConversionResourceError::InvalidInputTable {
                        name: conversion.custom_input_table.clone().unwrap_or_default(),
                    })?;
                ConverterInputStyle::Mapped(InputTableId::Named(name.clone()))
            }
        };
        self.default_keyboard_language = match conversion.keyboard_language {
            KeyboardLanguageConfig::None => KeyboardLanguage::None,
            KeyboardLanguageConfig::Japanese => KeyboardLanguage::Japanese,
            KeyboardLanguageConfig::EnglishUs => KeyboardLanguage::EnglishUs,
            KeyboardLanguageConfig::Greek => KeyboardLanguage::Greek,
        };
        Ok(())
    }

    fn apply_zenz_options(&mut self, config: &crate::config::ZenzConfig) {
        self.zenz_inference_limit = config.inference_limit;
        self.zenz_rich_candidates = config.rich_candidates;
        self.zenz_predictive_input = config.predictive_input;
        self.zenz_version = ZenzVersionConfig::V3(ZenzV3Config {
            profile: config.profile.clone(),
            topic: config.topic.clone(),
            style: config.style.clone(),
            preference: config.preference.clone(),
            enable_alignment_separator: config.enable_alignment_separator,
            ..ZenzV3Config::default()
        });
    }

    fn load_zenz_personalization(
        &mut self,
        config: &crate::config::ZenzConfig,
    ) -> Result<(), NGramError> {
        self.zenz_personalization = config
            .personalization
            .as_ref()
            .map(|personalization| {
                zenz::ZenzPersonalizationModels::load(
                    &personalization.base_ngram,
                    &personalization.personal_ngram,
                    personalization.alpha,
                )
            })
            .transpose()?;
        Ok(())
    }

    fn load_lm_typo(
        &mut self,
        config: &crate::config::LmTypoCorrectionConfig,
    ) -> Result<(), NGramError> {
        self.lm_typo_enabled = config.enabled;
        self.lm_typo_language_model = config.language_model;
        self.lm_typo_config = LmTypoConfig {
            beam_size: config.beam_size,
            top_k: config.top_k,
            n_best: config.n_best,
            max_steps: config.max_steps,
            substitution_cost: config.substitution_cost,
            deletion_cost: config.deletion_cost,
            transposition_cost: config.transposition_cost,
        };
        self.lm_typo_ngram =
            if config.enabled && config.language_model == LmTypoLanguageModel::Ngram {
                config
                    .ngram
                    .as_ref()
                    .map(|ngram| {
                        NGramLanguageModel::open(
                            &ngram.prefix,
                            &ngram.tokenizer,
                            ngram.n,
                            ngram.discount,
                        )
                    })
                    .transpose()?
            } else {
                None
            };
        Ok(())
    }

    pub fn handle(&mut self, envelope: protocol::Envelope) -> protocol::Envelope {
        let request_id = envelope.request_id;
        let session_id = envelope.session_id.clone();
        let payload = match envelope.payload {
            Some(payload) => payload,
            None => {
                return error_envelope(
                    request_id,
                    session_id,
                    Code::InvalidPayload,
                    "missing payload",
                );
            }
        };
        match payload {
            Payload::StartSession(start) => {
                if self
                    .sessions
                    .get(&session_id)
                    .is_some_and(|session| request_id <= session.last_request_id)
                {
                    return error_envelope(
                        request_id,
                        session_id,
                        Code::OutOfOrderRequest,
                        "request ID is not greater than the previous request",
                    );
                }
                let input_style = if protocol::InputStyle::try_from(start.input_style)
                    == Ok(protocol::InputStyle::Unspecified)
                {
                    Some(self.default_input_style.clone())
                } else {
                    input_style(start.input_style, &start.custom_input_table, &self.tables)
                };
                let Some(input_style) = input_style else {
                    return error_envelope(
                        request_id,
                        session_id,
                        Code::InvalidPayload,
                        "invalid input style",
                    );
                };
                let surrounding = match surrounding_context(start.surrounding_text.as_ref()) {
                    Ok(surrounding) => surrounding,
                    Err(message) => {
                        return error_envelope(
                            request_id,
                            session_id,
                            Code::InvalidPayload,
                            message,
                        );
                    }
                };
                let keyboard_language =
                    if protocol::KeyboardLanguage::try_from(start.keyboard_language)
                        == Ok(protocol::KeyboardLanguage::Unspecified)
                    {
                        Some(self.default_keyboard_language)
                    } else {
                        keyboard_language(start.keyboard_language)
                    };
                let Some(keyboard_language) = keyboard_language else {
                    return error_envelope(
                        request_id,
                        session_id,
                        Code::InvalidPayload,
                        "invalid keyboard language",
                    );
                };
                let mut conversion = ConversionSession::new();
                conversion.import_dynamic_user_dictionary(self.user_dictionary.clone(), Vec::new());
                if let Some(path) = &self.user_dictionary_directory
                    && let Err(error) = conversion.update_user_dictionary_path(path.clone())
                {
                    return error_envelope(
                        request_id,
                        session_id,
                        Code::Internal,
                        format!("user dictionary initialization failed: {error}"),
                    );
                }
                if let Some(provider) = &self.foreign_completion_provider {
                    conversion.set_foreign_completion_provider(provider.clone());
                }
                if let Some(memory) = &self.learning_memory
                    && let Err(error) = conversion.set_learning_memory(memory.clone())
                {
                    return error_envelope(
                        request_id,
                        session_id,
                        Code::Internal,
                        format!("learning memory initialization failed: {error}"),
                    );
                }
                self.sessions.insert(
                    session_id.clone(),
                    SessionState {
                        conversion,
                        input_style,
                        last_request_id: request_id,
                        selected_candidate: 0,
                        surrounding,
                        request_options: RequestOptions {
                            keyboard_language,
                            ..self.request_options.clone()
                        },
                        last_committed: None,
                        post_predictions: Vec::new(),
                        live_candidate: None,
                        typo_corrections: Vec::new(),
                        lm_typo_available: self.lm_typo_enabled,
                        learning_available: self.learning_available,
                        learning_writable: self.learning_writable,
                    },
                );
                state_envelope(
                    request_id,
                    session_id,
                    protocol::StateResponse {
                        reset: true,
                        lm_typo_available: self.lm_typo_enabled,
                        learning_available: self.learning_available,
                        learning_writable: self.learning_writable,
                        ..Default::default()
                    },
                )
            }
            Payload::StateResponse(_)
            | Payload::TypoCorrectionResponse(_)
            | Payload::ProtocolError(_) => error_envelope(
                request_id,
                session_id,
                Code::InvalidPayload,
                "response payload sent as a request",
            ),
            request => self.handle_session_request(request_id, session_id, request),
        }
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    fn handle_session_request(
        &mut self,
        request_id: u64,
        session_id: String,
        request: Payload,
    ) -> protocol::Envelope {
        let Some(mut session) = self.sessions.remove(&session_id) else {
            return error_envelope(
                request_id,
                session_id,
                Code::UnknownSession,
                "unknown session",
            );
        };
        if request_id <= session.last_request_id {
            self.sessions.insert(session_id.clone(), session);
            return error_envelope(
                request_id,
                session_id,
                Code::OutOfOrderRequest,
                "request ID is not greater than the previous request",
            );
        }
        session.last_request_id = request_id;
        if matches!(request, Payload::EndSession(_)) {
            return state_envelope(
                request_id,
                session_id,
                protocol::StateResponse {
                    reset: true,
                    lm_typo_available: session.lm_typo_available,
                    learning_available: session.learning_available,
                    learning_writable: session.learning_writable,
                    ..Default::default()
                },
            );
        }
        if matches!(request, Payload::RequestTypoCorrections(_)) {
            let result = self.request_typo_corrections(&session);
            match result {
                Ok(candidates) => {
                    session.typo_corrections.clone_from(&candidates);
                    self.sessions.insert(session_id.clone(), session);
                    return typo_correction_envelope(request_id, session_id, candidates);
                }
                Err((code, message)) => {
                    session.conversion.reset();
                    session.selected_candidate = 0;
                    session.last_committed = None;
                    session.post_predictions.clear();
                    session.live_candidate = None;
                    session.typo_corrections.clear();
                    self.sessions.insert(session_id.clone(), session);
                    return error_envelope(request_id, session_id, code, message);
                }
            }
        }

        let response = match request {
            Payload::ResetSession(_) => {
                session.conversion.reset();
                session.selected_candidate = 0;
                session.last_committed = None;
                session.post_predictions.clear();
                session.live_candidate = None;
                session.typo_corrections.clear();
                Ok(protocol::StateResponse {
                    reset: true,
                    lm_typo_available: session.lm_typo_available,
                    learning_available: session.learning_available,
                    learning_writable: session.learning_writable,
                    ..Default::default()
                })
            }
            Payload::KeyEvent(event) => {
                let updated = event
                    .surrounding_text
                    .as_ref()
                    .map(|surrounding| surrounding_context(Some(surrounding)))
                    .transpose();
                match updated {
                    Ok(Some(surrounding)) => {
                        session.surrounding = surrounding;
                        self.handle_key(&mut session, event)
                    }
                    Ok(None) => self.handle_key(&mut session, event),
                    Err(message) => Err((Code::InvalidPayload, message)),
                }
            }
            Payload::SelectCandidate(selection) => {
                self.select_candidate(&mut session, selection.index as usize)
            }
            Payload::SelectTypoCorrection(selection) => {
                self.select_typo_correction(&mut session, selection.index as usize)
            }
            Payload::ForgetCandidate(forget) => {
                self.forget_candidate(&mut session, forget.index as usize)
            }
            Payload::ResetLearning(_) => self.reset_learning(&mut session),
            Payload::PageCandidates(page) => {
                if !page_candidates(&mut session, page) {
                    self.sessions.insert(session_id.clone(), session);
                    return error_envelope(
                        request_id,
                        session_id,
                        Code::InvalidPayload,
                        "invalid candidate page request",
                    );
                }
                session.live_candidate = None;
                Ok(make_state(&session, true, String::new(), false))
            }
            Payload::CommitComposition(_) => {
                let commit = session.conversion.composing().surface();
                let consumed = !commit.is_empty();
                session.conversion.reset();
                session.selected_candidate = 0;
                session.last_committed = None;
                session.post_predictions.clear();
                session.live_candidate = None;
                session.typo_corrections.clear();
                Ok(protocol::StateResponse {
                    consumed,
                    commit,
                    reset: consumed,
                    lm_typo_available: session.lm_typo_available,
                    learning_available: session.learning_available,
                    learning_writable: session.learning_writable,
                    ..Default::default()
                })
            }
            Payload::StartSession(_)
            | Payload::EndSession(_)
            | Payload::RequestTypoCorrections(_)
            | Payload::StateResponse(_)
            | Payload::TypoCorrectionResponse(_)
            | Payload::ProtocolError(_) => {
                Err((Code::InvalidPayload, "invalid session request".into()))
            }
        };
        self.sessions.insert(session_id.clone(), session);
        match response {
            Ok(response) => state_envelope(request_id, session_id, response),
            Err((code, message)) => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.conversion.reset();
                    session.selected_candidate = 0;
                    session.last_committed = None;
                    session.post_predictions.clear();
                    session.live_candidate = None;
                    session.typo_corrections.clear();
                }
                error_envelope(request_id, session_id, code, message)
            }
        }
    }

    fn request_typo_corrections(
        &mut self,
        session: &SessionState,
    ) -> SessionRequestResult<Vec<beankey_converter::LmTypoCandidate>> {
        if !self.lm_typo_enabled {
            return Err((
                Code::InvalidPayload,
                "LM typo correction is disabled".into(),
            ));
        }
        let model: &mut dyn ZenzLanguageModel = match self.lm_typo_language_model {
            LmTypoLanguageModel::Zenz => self.zenz_model.as_deref_mut().ok_or_else(|| {
                (
                    Code::Internal,
                    "Zenz model is unavailable for LM typo correction".into(),
                )
            })?,
            LmTypoLanguageModel::Ngram => self.lm_typo_ngram.as_mut().ok_or_else(|| {
                (
                    Code::Internal,
                    "N-gram model is unavailable for LM typo correction".into(),
                )
            })?,
        };
        experimental_typo_correction(
            model,
            session.surrounding.left.as_deref().unwrap_or(""),
            session.conversion.composing(),
            &session.input_style,
            &self.tables,
            &self.lm_typo_config,
        )
        .map_err(|error| {
            (
                Code::Internal,
                format!("LM typo correction failed: {error}"),
            )
        })
    }

    fn handle_key(
        &mut self,
        session: &mut SessionState,
        event: protocol::KeyEvent,
    ) -> SessionRequestResult<protocol::StateResponse> {
        if event.release {
            return Ok(make_state(session, false, String::new(), false));
        }
        session.typo_corrections.clear();
        let post_candidate_count = session.post_predictions.len();
        match event.key_sym {
            KEY_BACKSPACE if !session.conversion.composing().is_empty() => {
                session.conversion.delete_backward(1, &self.tables);
            }
            KEY_DELETE if !session.conversion.composing().is_empty() => {
                session.conversion.delete_forward(1, &self.tables);
            }
            KEY_LEFT if !session.conversion.composing().is_empty() => {
                session.conversion.move_cursor(-1);
            }
            KEY_RIGHT if !session.conversion.composing().is_empty() => {
                session.conversion.move_cursor(1);
            }
            KEY_ESCAPE
                if !session.conversion.composing().is_empty() || post_candidate_count > 0 =>
            {
                session.conversion.reset();
                session.selected_candidate = 0;
                session.last_committed = None;
                session.post_predictions.clear();
                session.live_candidate = None;
                return Ok(protocol::StateResponse {
                    consumed: true,
                    reset: true,
                    lm_typo_available: session.lm_typo_available,
                    learning_available: session.learning_available,
                    learning_writable: session.learning_writable,
                    ..Default::default()
                });
            }
            KEY_RETURN | KEY_KP_ENTER if !session.conversion.composing().is_empty() => {
                if session.conversion.candidates().is_empty() {
                    let commit = session.conversion.composing().surface();
                    session.conversion.reset();
                    session.live_candidate = None;
                    return Ok(protocol::StateResponse {
                        consumed: true,
                        commit,
                        reset: true,
                        lm_typo_available: session.lm_typo_available,
                        learning_available: session.learning_available,
                        learning_writable: session.learning_writable,
                        ..Default::default()
                    });
                }
                return self.select_candidate(session, session.selected_candidate);
            }
            KEY_SPACE if !session.conversion.composing().is_empty() => {
                if !session.conversion.candidates().is_empty() {
                    session.live_candidate = None;
                    session.selected_candidate =
                        (session.selected_candidate + 1) % session.conversion.candidates().len();
                    return Ok(make_state(session, true, String::new(), false));
                }
            }
            KEY_SPACE if post_candidate_count > 0 => {
                session.selected_candidate =
                    (session.selected_candidate + 1) % post_candidate_count;
                return Ok(make_state(session, true, String::new(), false));
            }
            KEY_UP | KEY_DOWN if active_candidate_count(session) > 0 => {
                let count = active_candidate_count(session);
                session.live_candidate = None;
                session.selected_candidate = if event.key_sym == KEY_UP {
                    session
                        .selected_candidate
                        .checked_sub(1)
                        .unwrap_or(count - 1)
                } else {
                    (session.selected_candidate + 1) % count
                };
                return Ok(make_state(session, true, String::new(), false));
            }
            _ if valid_event_text(&event) => {
                session.last_committed = None;
                session.post_predictions.clear();
                session.live_candidate = None;
                if matches!(session.input_style, ConverterInputStyle::Mapped(_))
                    && (!event.input.is_empty() || !event.intention.is_empty())
                {
                    let input = if event.input.is_empty() {
                        event.text.clone()
                    } else {
                        event.input
                    };
                    let modifiers =
                        (event.modifiers & SHIFT_MODIFIER != 0).then_some(InputModifier::Shift);
                    session.conversion.insert_key(
                        (!event.intention.is_empty()).then_some(event.intention),
                        input,
                        modifiers,
                        session.input_style.clone(),
                        &self.tables,
                    );
                } else {
                    session.conversion.insert_str(
                        &event.text,
                        session.input_style.clone(),
                        &self.tables,
                    );
                }
            }
            _ if post_candidate_count > 0 => {
                session.last_committed = None;
                session.post_predictions.clear();
                session.live_candidate = None;
                return Ok(make_state(session, false, String::new(), true));
            }
            _ => return Ok(make_state(session, false, String::new(), false)),
        }

        session.selected_candidate = 0;
        session.live_candidate = None;
        if !session.conversion.composing().is_empty() {
            self.request_candidates(session)?;
        }
        Ok(make_state(session, true, String::new(), false))
    }

    fn select_candidate(
        &mut self,
        session: &mut SessionState,
        index: usize,
    ) -> SessionRequestResult<protocol::StateResponse> {
        session.typo_corrections.clear();
        if session.conversion.composing().is_empty() && !session.post_predictions.is_empty() {
            return self.select_post_prediction(session, index);
        }
        let selected = session
            .conversion
            .candidates()
            .get(index)
            .cloned()
            .ok_or_else(|| {
                (
                    Code::InvalidPayload,
                    format!("candidate index {index} is outside the current candidates"),
                )
            })?;
        let commit = session
            .conversion
            .select_candidate(index, &self.tables)
            .map_err(|error| match error {
                SelectionError::CandidateOutOfRange { .. } => (
                    Code::InvalidPayload,
                    format!("candidate selection failed: {error}"),
                ),
                SelectionError::Learning(_) => (
                    Code::Internal,
                    format!("candidate selection failed: {error}"),
                ),
            })?;
        session.conversion.commit_learning().map_err(|error| {
            (
                Code::Internal,
                format!("learning memory commit failed: {error}"),
            )
        })?;
        session.selected_candidate = 0;
        session.live_candidate = None;
        if !session.conversion.composing().is_empty() {
            session.last_committed = None;
            session.post_predictions.clear();
            self.request_candidates(session)?;
        } else {
            session.last_committed = Some(selected.clone());
            session.post_predictions = self.request_post_predictions(&selected)?;
        }
        let reset =
            session.post_predictions.is_empty() && session.conversion.composing().is_empty();
        Ok(make_state(session, true, commit, reset))
    }

    fn select_post_prediction(
        &mut self,
        session: &mut SessionState,
        index: usize,
    ) -> SessionRequestResult<protocol::StateResponse> {
        session.typo_corrections.clear();
        let prediction = session
            .post_predictions
            .get(index)
            .cloned()
            .ok_or_else(|| {
                (
                    Code::InvalidPayload,
                    format!(
                        "post-composition candidate index {index} is outside the current candidates"
                    ),
                )
            })?;
        let previous = session.last_committed.as_ref().ok_or_else(|| {
            (
                Code::Internal,
                "post-composition prediction has no committed candidate".into(),
            )
        })?;
        let joined = prediction.join(previous);
        if let Some(memory) = &self.learning_memory {
            memory
                .learn_post_prediction(previous, &prediction)
                .map_err(|error| {
                    (
                        Code::Internal,
                        format!("post-composition learning failed: {error}"),
                    )
                })?;
            memory.commit().map_err(|error| {
                (
                    Code::Internal,
                    format!("post-composition learning commit failed: {error}"),
                )
            })?;
        }
        session.selected_candidate = 0;
        session.live_candidate = None;
        session.last_committed = Some(joined.clone());
        session.post_predictions = if prediction.is_terminal {
            Vec::new()
        } else {
            self.request_post_predictions(&joined)?
        };
        let reset = session.post_predictions.is_empty();
        Ok(make_state(session, true, prediction.text, reset))
    }

    fn select_typo_correction(
        &mut self,
        session: &mut SessionState,
        index: usize,
    ) -> SessionRequestResult<protocol::StateResponse> {
        let correction = session
            .typo_corrections
            .get(index)
            .cloned()
            .ok_or_else(|| {
                (
                    Code::InvalidPayload,
                    format!("typo correction index {index} is outside the current candidates"),
                )
            })?;
        session.conversion.reset();
        session.selected_candidate = 0;
        session.last_committed = None;
        session.post_predictions.clear();
        session.live_candidate = None;
        session.typo_corrections.clear();
        Ok(protocol::StateResponse {
            consumed: true,
            commit: correction.converted_text,
            reset: true,
            lm_typo_available: session.lm_typo_available,
            learning_available: session.learning_available,
            learning_writable: session.learning_writable,
            ..Default::default()
        })
    }

    fn request_post_predictions(
        &self,
        candidate: &beankey_converter::Candidate,
    ) -> SessionRequestResult<Vec<PostCompositionPrediction>> {
        let predictions = match &self.text_replacer {
            Some(replacer) => {
                PostCompositionPredictor::with_text_replacer(&self.dictionary, replacer)
                    .predict(candidate)
            }
            None => PostCompositionPredictor::new(&self.dictionary).predict(candidate),
        };
        predictions.map_err(|error| {
            (
                Code::Internal,
                format!("post-composition prediction failed: {error}"),
            )
        })
    }

    fn forget_candidate(
        &mut self,
        session: &mut SessionState,
        index: usize,
    ) -> SessionRequestResult<protocol::StateResponse> {
        session.typo_corrections.clear();
        let candidate = session
            .conversion
            .candidates()
            .get(index)
            .cloned()
            .ok_or_else(|| {
                (
                    Code::InvalidPayload,
                    format!("candidate index {index} is outside the current candidates"),
                )
            })?;
        session
            .conversion
            .forget_learning(&candidate)
            .map_err(|error| {
                (
                    Code::Internal,
                    format!("learning memory forget failed: {error}"),
                )
            })?;
        session.selected_candidate = 0;
        session.live_candidate = None;
        self.request_candidates(session)?;
        Ok(make_state(session, true, String::new(), false))
    }

    fn reset_learning(
        &mut self,
        session: &mut SessionState,
    ) -> SessionRequestResult<protocol::StateResponse> {
        session.typo_corrections.clear();
        session.conversion.reset_learning().map_err(|error| {
            (
                Code::Internal,
                format!("learning memory reset failed: {error}"),
            )
        })?;
        session.selected_candidate = 0;
        session.live_candidate = None;
        if !session.conversion.composing().is_empty() {
            self.request_candidates(session)?;
        }
        Ok(make_state(session, true, String::new(), false))
    }

    fn request_candidates(&mut self, session: &mut SessionState) -> SessionRequestResult<()> {
        session.conversion.refresh_learning().map_err(|error| {
            (
                Code::Internal,
                format!("learning memory refresh failed: {error}"),
            )
        })?;
        let converter = NormalConverter::new(&self.dictionary);
        if let Some(model) = self.zenz_model.as_deref_mut() {
            let version = version_with_context(&self.zenz_version, &session.surrounding);
            zenz::convert(
                &mut session.conversion,
                &converter,
                &self.tables,
                model,
                zenz::ZenzConversionOptions {
                    version: &version,
                    request_rich_candidates: self.zenz_rich_candidates,
                    inference_limit: self.zenz_inference_limit,
                    personalization: self.zenz_personalization.as_ref(),
                },
            )
            .map_err(|error| {
                (
                    Code::Internal,
                    format!("Zenz candidate generation failed: {error}"),
                )
            })?;
            let prediction_override = if self.zenz_predictive_input
                && session.request_options.japanese_prediction != PredictionMode::Disabled
            {
                Some(
                    session
                        .conversion
                        .request_zenz_prediction(
                            &converter,
                            &self.tables,
                            model,
                            &version,
                            session.surrounding.left.as_deref().unwrap_or(""),
                        )
                        .map_err(|error| {
                            (
                                Code::Internal,
                                format!("Zenz input prediction failed: {error}"),
                            )
                        })?,
                )
            } else {
                None
            };
            session
                .conversion
                .finalize_zenz_request_with_prediction_override(
                    &converter,
                    &self.tables,
                    session.request_options.clone(),
                    prediction_override,
                )
                .map_err(|error| {
                    (
                        Code::Internal,
                        format!("candidate assembly failed: {error}"),
                    )
                })?;
        } else {
            session
                .conversion
                .request(&converter, &self.tables, session.request_options.clone())
                .map_err(|error| {
                    (
                        Code::Internal,
                        format!("candidate generation failed: {error}"),
                    )
                })?;
        }
        let live_candidate = if self.live_conversion && session.conversion.composing().is_at_end() {
            session
                .conversion
                .request_live_conversion(&converter, &self.tables)
                .map_err(|error| (Code::Internal, format!("live conversion failed: {error}")))?
        } else {
            None
        };
        if let Some(live_candidate) = &live_candidate
            && let Some(index) = session
                .conversion
                .candidates()
                .iter()
                .position(|candidate| {
                    candidate.text == live_candidate.text
                        && candidate.composing_count == live_candidate.composing_count
                })
        {
            session.selected_candidate = index;
        }
        session.live_candidate = live_candidate;
        Ok(())
    }
}

fn valid_event_text(event: &protocol::KeyEvent) -> bool {
    !event.text.is_empty()
        && !event.text.chars().any(char::is_control)
        && !event.input.chars().any(char::is_control)
        && !event.intention.chars().any(char::is_control)
}

fn surrounding_context(
    surrounding: Option<&protocol::SurroundingText>,
) -> Result<SurroundingContext, String> {
    let Some(surrounding) = surrounding.filter(|surrounding| surrounding.available) else {
        return Ok(SurroundingContext::default());
    };
    let character_count = surrounding.text.chars().count();
    let cursor = surrounding.cursor as usize;
    let anchor = surrounding.anchor as usize;
    if cursor > character_count || anchor > character_count {
        return Err(format!(
            "surrounding text cursor {cursor} and anchor {anchor} exceed {character_count} characters"
        ));
    }
    let selection_start = cursor.min(anchor);
    let selection_end = cursor.max(anchor);
    Ok(SurroundingContext {
        left: Some(surrounding.text.chars().take(selection_start).collect()),
        right: Some(surrounding.text.chars().skip(selection_end).collect()),
    })
}

fn version_with_context(
    version: &ZenzVersionConfig,
    surrounding: &SurroundingContext,
) -> ZenzVersionConfig {
    match version.clone() {
        ZenzVersionConfig::V2(mut config) => {
            config.left_context.clone_from(&surrounding.left);
            ZenzVersionConfig::V2(config)
        }
        ZenzVersionConfig::V3(mut config) => {
            config.left_context.clone_from(&surrounding.left);
            config.right_context.clone_from(&surrounding.right);
            ZenzVersionConfig::V3(config)
        }
    }
}

fn input_style(
    value: i32,
    custom_table: &str,
    tables: &InputTableRegistry,
) -> Option<ConverterInputStyle> {
    match protocol::InputStyle::try_from(value).ok()? {
        protocol::InputStyle::Unspecified => None,
        protocol::InputStyle::Direct => Some(ConverterInputStyle::Direct),
        protocol::InputStyle::RomanToKana => Some(ConverterInputStyle::RomanToKana),
        protocol::InputStyle::Azik => Some(ConverterInputStyle::Mapped(InputTableId::DefaultAzik)),
        protocol::InputStyle::KanaJis => {
            Some(ConverterInputStyle::Mapped(InputTableId::DefaultKanaJis))
        }
        protocol::InputStyle::KanaUs => {
            Some(ConverterInputStyle::Mapped(InputTableId::DefaultKanaUs))
        }
        protocol::InputStyle::Custom => tables
            .contains(custom_table)
            .then(|| ConverterInputStyle::Mapped(InputTableId::Named(custom_table.into()))),
    }
}

fn keyboard_language(value: i32) -> Option<KeyboardLanguage> {
    match protocol::KeyboardLanguage::try_from(value).ok()? {
        protocol::KeyboardLanguage::Unspecified => None,
        protocol::KeyboardLanguage::Japanese => Some(KeyboardLanguage::Japanese),
        protocol::KeyboardLanguage::EnglishUs => Some(KeyboardLanguage::EnglishUs),
        protocol::KeyboardLanguage::Greek => Some(KeyboardLanguage::Greek),
    }
}

fn prediction_mode(value: PredictionConfig) -> PredictionMode {
    match value {
        PredictionConfig::Automatic => PredictionMode::Automatic,
        PredictionConfig::Manual => PredictionMode::Manual,
        PredictionConfig::Disabled => PredictionMode::Disabled,
    }
}

fn default_request_options() -> RequestOptions {
    RequestOptions {
        foreign_prediction: PredictionMode::Automatic,
        version_string: Some(format!("beankey {}", env!("CARGO_PKG_VERSION"))),
        ..RequestOptions::default()
    }
}

fn typo_correction_mode(value: TypoCorrectionConfig) -> TypoCorrectionMode {
    match value {
        TypoCorrectionConfig::Enabled => TypoCorrectionMode::Enabled,
        TypoCorrectionConfig::Automatic => TypoCorrectionMode::Automatic,
        TypoCorrectionConfig::Disabled => TypoCorrectionMode::Disabled,
    }
}

fn to_katakana(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{3041}'..='\u{3096}' => {
                char::from_u32(u32::from(character) + 96).expect("katakana scalar is valid")
            }
            _ => character,
        })
        .collect()
}

fn page_candidates(session: &mut SessionState, page: protocol::PageCandidates) -> bool {
    let count = active_candidate_count(session);
    if page.page_size == 0 {
        return false;
    }
    if count == 0 {
        return true;
    }
    let page_size = page.page_size as usize;
    session.selected_candidate =
        match protocol::page_candidates::Direction::try_from(page.direction) {
            Ok(protocol::page_candidates::Direction::Previous) => {
                session.selected_candidate.saturating_sub(page_size)
            }
            Ok(protocol::page_candidates::Direction::Next) => {
                (session.selected_candidate + page_size).min(count - 1)
            }
            _ => return false,
        };
    true
}

fn active_candidate_count(session: &SessionState) -> usize {
    if session.conversion.composing().is_empty() && !session.post_predictions.is_empty() {
        session.post_predictions.len()
    } else {
        session.conversion.candidates().len()
    }
}

fn make_state(
    session: &SessionState,
    consumed: bool,
    commit: String,
    reset: bool,
) -> protocol::StateResponse {
    let (preedit, preedit_cursor) = session.live_candidate.as_ref().map_or_else(
        || {
            (
                session.conversion.composing().surface(),
                session.conversion.composing().cursor() as u32,
            )
        },
        |candidate| {
            (
                candidate.text.clone(),
                candidate.text.chars().count().min(u32::MAX as usize) as u32,
            )
        },
    );
    protocol::StateResponse {
        consumed,
        preedit,
        preedit_cursor,
        candidates: if session.conversion.composing().is_empty()
            && !session.post_predictions.is_empty()
        {
            session
                .post_predictions
                .iter()
                .map(post_prediction_to_protocol)
                .collect()
        } else {
            session
                .conversion
                .candidates()
                .iter()
                .map(candidate_to_protocol)
                .collect()
        },
        selected_candidate: session.selected_candidate as i32,
        commit,
        reset,
        lm_typo_available: session.lm_typo_available,
        learning_available: session.learning_available,
        learning_writable: session.learning_writable,
    }
}

fn post_prediction_to_protocol(prediction: &PostCompositionPrediction) -> protocol::Candidate {
    protocol::Candidate {
        text: prediction.text.clone(),
        value: prediction.value,
        composing_count: None,
        actions: Vec::new(),
    }
}

fn candidate_to_protocol(candidate: &beankey_converter::Candidate) -> protocol::Candidate {
    protocol::Candidate {
        text: candidate.text.clone(),
        value: candidate.value,
        composing_count: Some(composing_count_to_protocol(&candidate.composing_count)),
        actions: candidate
            .actions
            .iter()
            .map(|action| match action {
                CompleteAction::MoveCursor(count) => protocol::CursorAction {
                    r#move: i32::try_from(*count).unwrap_or(if *count < 0 {
                        i32::MIN
                    } else {
                        i32::MAX
                    }),
                },
            })
            .collect(),
    }
}

fn composing_count_to_protocol(count: &ConverterComposingCount) -> protocol::ComposingCount {
    let count = match count {
        ConverterComposingCount::Input(count) => {
            Count::Input((*count).min(u32::MAX as usize) as u32)
        }
        ConverterComposingCount::Surface(count) => {
            Count::Surface((*count).min(u32::MAX as usize) as u32)
        }
        ConverterComposingCount::Composite(left, right) => {
            Count::Composite(Box::new(protocol::CompositeComposingCount {
                left: Some(Box::new(composing_count_to_protocol(left))),
                right: Some(Box::new(composing_count_to_protocol(right))),
            }))
        }
    };
    protocol::ComposingCount { count: Some(count) }
}

fn state_envelope(
    request_id: u64,
    session_id: String,
    response: protocol::StateResponse,
) -> protocol::Envelope {
    protocol::Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        session_id,
        payload: Some(Payload::StateResponse(response)),
        trace: Vec::new(),
    }
}

fn typo_correction_envelope(
    request_id: u64,
    session_id: String,
    candidates: Vec<beankey_converter::LmTypoCandidate>,
) -> protocol::Envelope {
    protocol::Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        session_id,
        payload: Some(Payload::TypoCorrectionResponse(
            protocol::TypoCorrectionResponse {
                candidates: candidates
                    .into_iter()
                    .map(|candidate| protocol::TypoCorrectionCandidate {
                        corrected_input: candidate.corrected_input,
                        converted_text: candidate.converted_text,
                        score: candidate.score,
                        language_model_score: candidate.lm_score,
                        channel_cost: candidate.channel_cost,
                        prominence: candidate.prominence,
                    })
                    .collect(),
            },
        )),
        trace: Vec::new(),
    }
}

fn error_envelope(
    request_id: u64,
    session_id: String,
    code: Code,
    message: impl Into<String>,
) -> protocol::Envelope {
    protocol::Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        session_id,
        payload: Some(Payload::ProtocolError(protocol::ProtocolError {
            code: code as i32,
            message: message.into(),
        })),
        trace: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use beankey_converter::{ForeignLanguage, ZenzV3Config, ZenzVersionConfig};

    use super::*;

    struct GreekCompleter;

    impl ForeignCompletionProvider for GreekCompleter {
        fn completions(&self, language: ForeignLanguage, input: &str) -> Vec<String> {
            if language == ForeignLanguage::Greek && input == "καλ" {
                vec!["καλά".into()]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn distinguishes_read_only_learning_management_from_candidate_forgetting() {
        let dictionary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("data/azooKey_dictionary_storage/Dictionary");
        let state = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dictionary).unwrap();
        engine
            .load_learning(
                state.path(),
                &LearningConfig {
                    mode: LearningModeConfig::OnlyOutput,
                    ..Default::default()
                },
            )
            .unwrap();

        let response = engine.handle(protocol::Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1,
            session_id: "read-only-learning".into(),
            payload: Some(Payload::StartSession(protocol::StartSession {
                input_style: protocol::InputStyle::RomanToKana as i32,
                surrounding_text: None,
                keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
                custom_input_table: String::new(),
            })),
            trace: Vec::new(),
        });
        let Some(Payload::StateResponse(response)) = response.payload else {
            panic!("session start did not return state");
        };
        assert!(response.learning_available);
        assert!(!response.learning_writable);
    }

    #[test]
    fn uses_the_configured_keyboard_language_for_unspecified_sessions() {
        let dictionary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("data/azooKey_dictionary_storage/Dictionary");
        let mut engine = Engine::open(dictionary).unwrap();
        engine.foreign_completion_provider = Some(Arc::new(GreekCompleter));
        engine
            .apply_conversion_options(&ConversionConfig {
                input_style: InputStyleConfig::Direct,
                keyboard_language: KeyboardLanguageConfig::Greek,
                ..Default::default()
            })
            .unwrap();
        let envelope = |request_id, payload| protocol::Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            session_id: "greek".into(),
            payload: Some(payload),
            trace: Vec::new(),
        };
        engine.handle(envelope(
            1,
            Payload::StartSession(protocol::StartSession {
                input_style: protocol::InputStyle::Unspecified as i32,
                surrounding_text: None,
                keyboard_language: protocol::KeyboardLanguage::Unspecified as i32,
                custom_input_table: String::new(),
            }),
        ));

        let response = engine.handle(envelope(
            2,
            Payload::KeyEvent(protocol::KeyEvent {
                text: "καλ".into(),
                ..Default::default()
            }),
        ));
        let Payload::StateResponse(state) = response.payload.unwrap() else {
            panic!("conversion did not return state");
        };

        assert!(
            state
                .candidates
                .iter()
                .any(|candidate| candidate.text == "καλά")
        );
    }

    #[test]
    fn splits_fcitx_surrounding_text_around_the_selection() {
        let context = surrounding_context(Some(&protocol::SurroundingText {
            available: true,
            text: "甲😀選択乙".into(),
            cursor: 4,
            anchor: 2,
        }))
        .unwrap();

        assert_eq!(context.left.as_deref(), Some("甲😀"));
        assert_eq!(context.right.as_deref(), Some("乙"));
    }

    #[test]
    fn rejects_out_of_range_surrounding_text_positions() {
        let error = surrounding_context(Some(&protocol::SurroundingText {
            available: true,
            text: "甲😀".into(),
            cursor: 3,
            anchor: 2,
        }))
        .unwrap_err();

        assert!(error.contains("exceed 2 characters"));
    }

    #[test]
    fn applies_dynamic_context_without_replacing_zenz_conditions() {
        let version = ZenzVersionConfig::V3(ZenzV3Config {
            profile: Some("profile".into()),
            left_context: Some("stale left".into()),
            right_context: Some("stale right".into()),
            ..Default::default()
        });
        let updated = version_with_context(
            &version,
            &SurroundingContext {
                left: Some("left".into()),
                right: Some("right".into()),
            },
        );

        assert_eq!(
            updated,
            ZenzVersionConfig::V3(ZenzV3Config {
                profile: Some("profile".into()),
                left_context: Some("left".into()),
                right_context: Some("right".into()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn applies_static_zenz_configuration() {
        let dictionary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("data/azooKey_dictionary_storage/Dictionary");
        let mut engine = Engine::open(dictionary).unwrap();
        engine.apply_zenz_options(&crate::ZenzConfig {
            inference_limit: 7,
            rich_candidates: true,
            predictive_input: true,
            profile: Some("profile".into()),
            topic: Some("topic".into()),
            style: Some("style".into()),
            preference: Some("preference".into()),
            enable_alignment_separator: true,
            personalization: None,
        });

        assert_eq!(engine.zenz_inference_limit, 7);
        assert!(engine.zenz_rich_candidates);
        assert!(engine.zenz_predictive_input);
        assert_eq!(
            engine.zenz_version,
            ZenzVersionConfig::V3(ZenzV3Config {
                profile: Some("profile".into()),
                topic: Some("topic".into()),
                style: Some("style".into()),
                preference: Some("preference".into()),
                enable_alignment_separator: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn loads_fixed_format_personalization_models() {
        let dictionary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("data/azooKey_dictionary_storage/Dictionary");
        let ngram = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../converter/tests/data/ngram");
        let mut engine = Engine::open(dictionary).unwrap();
        let config = crate::ZenzConfig {
            personalization: Some(crate::PersonalizationConfig {
                base_ngram: ngram.join("lm"),
                personal_ngram: ngram.join("personal"),
                alpha: 0.5,
            }),
            ..Default::default()
        };

        engine.load_zenz_personalization(&config).unwrap();

        assert!(engine.zenz_personalization.is_some());
    }

    #[test]
    fn returns_lm_typo_candidates_without_mutating_composition() {
        let dictionary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("data/azooKey_dictionary_storage/Dictionary");
        let ngram =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../converter/tests/data/ngram/lm");
        let tokenizer = std::env::var_os("BEANKEY_TEST_ZENZ_TOKENIZER")
            .expect("BEANKEY_TEST_ZENZ_TOKENIZER must point to tokenizer.json");
        let mut engine = Engine::open(dictionary).unwrap();
        engine
            .load_lm_typo(&crate::LmTypoCorrectionConfig {
                enabled: true,
                language_model: crate::LmTypoLanguageModel::Ngram,
                ngram: Some(crate::TypoNGramConfig {
                    prefix: ngram,
                    tokenizer: tokenizer.into(),
                    n: 2,
                    discount: 0.75,
                }),
                max_steps: Some(0),
                ..Default::default()
            })
            .unwrap();
        let envelope = |request_id, payload| protocol::Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            session_id: "typo".into(),
            payload: Some(payload),
            trace: Vec::new(),
        };
        engine.handle(envelope(
            1,
            Payload::StartSession(protocol::StartSession {
                input_style: protocol::InputStyle::RomanToKana as i32,
                surrounding_text: None,
                keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
                custom_input_table: String::new(),
            }),
        ));
        engine.handle(envelope(
            2,
            Payload::KeyEvent(protocol::KeyEvent {
                text: "kana".into(),
                ..Default::default()
            }),
        ));

        let correction = engine.handle(envelope(
            3,
            Payload::RequestTypoCorrections(protocol::RequestTypoCorrections {}),
        ));
        let Payload::TypoCorrectionResponse(correction) = correction.payload.unwrap() else {
            panic!("LM typo correction did not return its dedicated response");
        };
        assert_eq!(correction.candidates[0].corrected_input, "kana");
        let expected_commit = correction.candidates[0].converted_text.clone();

        let invalid = engine.handle(envelope(
            4,
            Payload::SelectTypoCorrection(protocol::SelectTypoCorrection {
                index: correction.candidates.len() as u32,
            }),
        ));
        assert!(matches!(invalid.payload, Some(Payload::ProtocolError(_))));
        let stale = engine.handle(envelope(
            5,
            Payload::SelectTypoCorrection(protocol::SelectTypoCorrection { index: 0 }),
        ));
        assert!(matches!(stale.payload, Some(Payload::ProtocolError(_))));

        engine.handle(envelope(
            6,
            Payload::KeyEvent(protocol::KeyEvent {
                text: "kana".into(),
                ..Default::default()
            }),
        ));
        engine.handle(envelope(
            7,
            Payload::RequestTypoCorrections(protocol::RequestTypoCorrections {}),
        ));
        let selected = engine.handle(envelope(
            8,
            Payload::SelectTypoCorrection(protocol::SelectTypoCorrection { index: 0 }),
        ));
        let Payload::StateResponse(selected) = selected.payload.unwrap() else {
            panic!("LM typo correction selection did not return state");
        };
        assert_eq!(selected.commit, expected_commit);
        assert!(selected.consumed);
        assert!(selected.reset);
        assert!(selected.preedit.is_empty());
    }
}
