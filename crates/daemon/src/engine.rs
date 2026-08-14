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
    LearningMode, NormalConverter, PostCompositionPrediction, PostCompositionPredictor,
    PredictionMode, RequestOptions, SelectionError, TextReplacer, TextReplacerError,
    ZenzLanguageModel, ZenzVersionConfig,
};
use beankey_llama::LlamaError;
use serde::Deserialize;

use crate::config::{ConversionConfig, DaemonConfig};
use crate::protocol::composing_count::Count;
use crate::protocol::envelope::Payload;
use crate::protocol::protocol_error::Code;
use crate::zenz;
use crate::{LlamaModel, PROTOCOL_VERSION, protocol};

const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
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
    foreign_completion_provider: Option<Arc<dyn ForeignCompletionProvider>>,
    learning_memory: Option<LearningMemory>,
    text_replacer: Option<TextReplacer>,
    user_dictionary: Vec<DictionaryEntry>,
    user_dictionary_directory: Option<PathBuf>,
}

#[derive(Debug)]
pub enum EngineOpenError {
    Dictionary(DictionaryError),
    Llama(LlamaError),
    Hunspell(HunspellError),
    Learning(LearningError),
    TextReplacer(TextReplacerError),
    ConversionResource(ConversionResourceError),
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
            foreign_completion_provider: None,
            learning_memory: None,
            text_replacer: None,
            user_dictionary: Vec::new(),
            user_dictionary_directory: None,
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
        engine.load_learning(learning_directory)?;
        engine.load_text_replacer(&config.emoji_dictionary)?;
        engine.load_conversion_resources(&config.conversion)?;
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
        engine.load_learning(learning_directory)?;
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

    fn load_learning(&mut self, learning_directory: impl AsRef<Path>) -> Result<(), LearningError> {
        self.learning_memory = Some(LearningMemory::open(
            learning_directory.as_ref().to_path_buf(),
            LearningMode::InputAndOutput,
            65_536,
        )?);
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
                let Some(input_style) =
                    input_style(start.input_style, &start.custom_input_table, &self.tables)
                else {
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
                let Some(keyboard_language) = keyboard_language(start.keyboard_language) else {
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
                            foreign_prediction: PredictionMode::Automatic,
                            keyboard_language,
                            version_string: Some(format!("beankey {}", env!("CARGO_PKG_VERSION"))),
                            ..RequestOptions::default()
                        },
                        last_committed: None,
                        post_predictions: Vec::new(),
                    },
                );
                state_envelope(
                    request_id,
                    session_id,
                    protocol::StateResponse {
                        reset: true,
                        ..Default::default()
                    },
                )
            }
            Payload::StateResponse(_) | Payload::ProtocolError(_) => error_envelope(
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
                    ..Default::default()
                },
            );
        }

        let response = match request {
            Payload::ResetSession(_) => {
                session.conversion.reset();
                session.selected_candidate = 0;
                session.last_committed = None;
                session.post_predictions.clear();
                Ok(protocol::StateResponse {
                    reset: true,
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
                Ok(make_state(&session, true, String::new(), false))
            }
            Payload::CommitComposition(_) => {
                let commit = session.conversion.composing().surface();
                let consumed = !commit.is_empty();
                session.conversion.reset();
                session.selected_candidate = 0;
                session.last_committed = None;
                session.post_predictions.clear();
                Ok(protocol::StateResponse {
                    consumed,
                    commit,
                    reset: consumed,
                    ..Default::default()
                })
            }
            Payload::StartSession(_)
            | Payload::EndSession(_)
            | Payload::StateResponse(_)
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
                }
                error_envelope(request_id, session_id, code, message)
            }
        }
    }

    fn handle_key(
        &mut self,
        session: &mut SessionState,
        event: protocol::KeyEvent,
    ) -> SessionRequestResult<protocol::StateResponse> {
        if event.release {
            return Ok(make_state(session, false, String::new(), false));
        }
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
                return Ok(protocol::StateResponse {
                    consumed: true,
                    reset: true,
                    ..Default::default()
                });
            }
            KEY_RETURN if !session.conversion.composing().is_empty() => {
                if session.conversion.candidates().is_empty() {
                    let commit = session.conversion.composing().surface();
                    session.conversion.reset();
                    return Ok(protocol::StateResponse {
                        consumed: true,
                        commit,
                        reset: true,
                        ..Default::default()
                    });
                }
                return self.select_candidate(session, session.selected_candidate);
            }
            KEY_RETURN if post_candidate_count > 0 => {
                return self.select_post_prediction(session, session.selected_candidate);
            }
            KEY_SPACE if !session.conversion.composing().is_empty() => {
                if !session.conversion.candidates().is_empty() {
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
            _ if !event.text.is_empty() => {
                session.last_committed = None;
                session.post_predictions.clear();
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
                return Ok(make_state(session, false, String::new(), true));
            }
            _ => return Ok(make_state(session, false, String::new(), false)),
        }

        session.selected_candidate = 0;
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
        session.last_committed = Some(joined.clone());
        session.post_predictions = if prediction.is_terminal {
            Vec::new()
        } else {
            self.request_post_predictions(&joined)?
        };
        let reset = session.post_predictions.is_empty();
        Ok(make_state(session, true, prediction.text, reset))
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
        self.request_candidates(session)?;
        Ok(make_state(session, true, String::new(), false))
    }

    fn reset_learning(
        &mut self,
        session: &mut SessionState,
    ) -> SessionRequestResult<protocol::StateResponse> {
        session.conversion.reset_learning().map_err(|error| {
            (
                Code::Internal,
                format!("learning memory reset failed: {error}"),
            )
        })?;
        session.selected_candidate = 0;
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
                &version,
                self.zenz_rich_candidates,
                self.zenz_inference_limit,
            )
            .map_err(|error| {
                (
                    Code::Internal,
                    format!("Zenz candidate generation failed: {error}"),
                )
            })?;
            session
                .conversion
                .finalize_zenz_request(&converter, &self.tables, session.request_options.clone())
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
        Ok(())
    }
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
        protocol::KeyboardLanguage::Unspecified | protocol::KeyboardLanguage::Japanese => {
            Some(KeyboardLanguage::Japanese)
        }
        protocol::KeyboardLanguage::EnglishUs => Some(KeyboardLanguage::EnglishUs),
        protocol::KeyboardLanguage::Greek => Some(KeyboardLanguage::Greek),
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
    protocol::StateResponse {
        consumed,
        preedit: session.conversion.composing().surface(),
        preedit_cursor: session.conversion.composing().cursor() as u32,
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
    use beankey_converter::{ZenzV3Config, ZenzVersionConfig};

    use super::*;

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
}
