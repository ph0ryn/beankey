use std::error::Error;
use std::fmt;
use std::path::Path;

use beankey_converter::{
    Candidate, CandidateEvaluation, ConversionSession, DictionaryError, DictionaryMetadata,
    EfficientNGram, InputTableRegistry, NGramError, NormalConverter, PrefixConstraint,
    ZenzEvaluationRequest, ZenzEvaluator, ZenzInferenceError, ZenzInferenceSequence,
    ZenzLanguageModel, ZenzPersonalization, ZenzVersionConfig, to_katakana,
};
use beankey_llama::{LlamaContext, LlamaError, LlamaSequence};

pub const DEFAULT_INFERENCE_LIMIT: usize = 10;
const PERSONALIZATION_N: usize = 5;
const PERSONALIZATION_DISCOUNT: f64 = 0.75;

pub(crate) struct ZenzPersonalizationModels {
    alpha: f32,
    base: EfficientNGram,
    personal: EfficientNGram,
}

pub(crate) struct ZenzConversionOptions<'a> {
    pub(crate) version: &'a ZenzVersionConfig,
    pub(crate) request_rich_candidates: bool,
    pub(crate) inference_limit: usize,
    pub(crate) personalization: Option<&'a ZenzPersonalizationModels>,
}

impl ZenzPersonalizationModels {
    pub(crate) fn load(
        base: impl AsRef<Path>,
        personal: impl AsRef<Path>,
        alpha: f32,
    ) -> Result<Self, NGramError> {
        Ok(Self {
            alpha,
            base: EfficientNGram::open(base, PERSONALIZATION_N, PERSONALIZATION_DISCOUNT)?,
            personal: EfficientNGram::open(personal, PERSONALIZATION_N, PERSONALIZATION_DISCOUNT)?,
        })
    }

    fn request(&self) -> ZenzPersonalization<'_> {
        ZenzPersonalization {
            alpha: self.alpha,
            base: &self.base,
            personal: &self.personal,
        }
    }
}

pub struct LlamaModel {
    context: LlamaContext,
}

impl LlamaModel {
    pub fn load(
        model_path: impl AsRef<Path>,
        backend_directory: impl AsRef<Path>,
    ) -> Result<Self, LlamaError> {
        Ok(Self {
            context: LlamaContext::load(model_path, backend_directory)?,
        })
    }
}

impl ZenzLanguageModel for LlamaModel {
    fn vocabulary_size(&self) -> usize {
        self.context.vocabulary_size()
    }

    fn eos_token(&self) -> i32 {
        self.context.eos_token()
    }

    fn tokenize(&mut self, text: &str, add_special: bool) -> Result<Vec<i32>, ZenzInferenceError> {
        self.context
            .tokenize(text, add_special)
            .map_err(inference_error)
    }

    fn token_to_piece(&mut self, token: i32) -> Result<Vec<u8>, ZenzInferenceError> {
        self.context.token_to_piece(token).map_err(inference_error)
    }

    fn next_logits(&mut self, tokens: &[i32]) -> Result<Vec<f32>, ZenzInferenceError> {
        self.context.next_logits(tokens).map_err(inference_error)
    }

    fn logits_for_suffix(
        &mut self,
        tokens: &[i32],
        start_index: usize,
        sequence: ZenzInferenceSequence,
    ) -> Result<Vec<f32>, ZenzInferenceError> {
        let sequence = match sequence {
            ZenzInferenceSequence::Evaluation => LlamaSequence::Evaluation,
            ZenzInferenceSequence::InputPrediction => LlamaSequence::InputPrediction,
        };
        self.context
            .logits(tokens, start_index, sequence)
            .map_err(inference_error)
    }
}

fn inference_error(error: LlamaError) -> ZenzInferenceError {
    ZenzInferenceError(error.to_string())
}

#[derive(Debug)]
pub enum ZenzConversionError {
    Dictionary(DictionaryError),
    Inference(ZenzInferenceError),
}

impl fmt::Display for ZenzConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dictionary(error) => error.fmt(formatter),
            Self::Inference(error) => error.fmt(formatter),
        }
    }
}

impl Error for ZenzConversionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Dictionary(error) => Some(error),
            Self::Inference(error) => Some(error),
        }
    }
}

impl From<DictionaryError> for ZenzConversionError {
    fn from(value: DictionaryError) -> Self {
        Self::Dictionary(value)
    }
}

impl From<ZenzInferenceError> for ZenzConversionError {
    fn from(value: ZenzInferenceError) -> Self {
        Self::Inference(value)
    }
}

pub fn convert(
    session: &mut ConversionSession,
    converter: &NormalConverter<'_>,
    tables: &InputTableRegistry,
    model: &mut dyn ZenzLanguageModel,
    evaluator: &mut ZenzEvaluator,
    options: ZenzConversionOptions<'_>,
) -> Result<(), ZenzConversionError> {
    let input = to_katakana(&session.composing().surface());
    let input_cursor_position = Some(session.composing().cursor());
    let mut constraint = PrefixConstraint::default();

    for inference in 0..=options.inference_limit {
        let draft = session
            .request_zenz_draft(
                converter,
                tables,
                if constraint.is_empty() {
                    2
                } else if options.request_rich_candidates {
                    3
                } else {
                    1
                },
                &constraint,
            )?
            .to_vec();
        let Some(candidate) = draft.first() else {
            session.set_zenz_candidates(draft);
            return Ok(());
        };
        if inference == options.inference_limit {
            session.set_zenz_candidates(draft);
            return Ok(());
        }

        let evaluation = evaluator.evaluate(
            model,
            ZenzEvaluationRequest {
                input: &input,
                input_cursor_position,
                candidate,
                request_rich_candidates: options.request_rich_candidates,
                prefix_constraint: &constraint,
                personalization: options
                    .personalization
                    .map(ZenzPersonalizationModels::request),
                version: options.version,
            },
        )?;
        match evaluation {
            CandidateEvaluation::Pass { alternatives, .. } => {
                let mut resolved = draft;
                for alternative in alternatives
                    .into_iter()
                    .rev()
                    .filter(|alternative| alternative.probability_ratio > 0.25)
                {
                    let alternative_constraint = PrefixConstraint::normalized(
                        alternative.bytes,
                        false,
                        constraint.ignore_memory_and_user_dictionary,
                    );
                    let existing = resolved
                        .iter()
                        .filter(|candidate| candidate_satisfies(candidate, &alternative_constraint))
                        .max_by(|left, right| left.value.total_cmp(&right.value))
                        .cloned();
                    let alternative = if existing.is_some() {
                        existing
                    } else if alternative.probability_ratio > 0.5 {
                        session
                            .request_zenz_draft(converter, tables, 3, &alternative_constraint)?
                            .iter()
                            .max_by(|left, right| left.value.total_cmp(&right.value))
                            .cloned()
                    } else {
                        None
                    };
                    if let Some(alternative) = alternative
                        && !resolved.iter().any(|item| item.text == alternative.text)
                    {
                        resolved.insert(1.min(resolved.len()), alternative);
                    }
                }
                session.set_zenz_candidates(resolved);
                return Ok(());
            }
            CandidateEvaluation::FixRequired(bytes) => {
                let next = PrefixConstraint::normalized(
                    bytes,
                    false,
                    constraint.ignore_memory_and_user_dictionary,
                );
                if next == constraint {
                    if !constraint.ignore_memory_and_user_dictionary
                        && candidate_uses_personal_dictionary(candidate)
                    {
                        constraint.ignore_memory_and_user_dictionary = true;
                        continue;
                    }
                    session.set_zenz_candidates(draft);
                    return Ok(());
                }
                constraint = next;
            }
            CandidateEvaluation::WholeResult(result) => {
                let next = PrefixConstraint::normalized(
                    result.into_bytes(),
                    true,
                    constraint.ignore_memory_and_user_dictionary,
                );
                if next == constraint {
                    if !constraint.ignore_memory_and_user_dictionary
                        && candidate_uses_personal_dictionary(candidate)
                    {
                        constraint.ignore_memory_and_user_dictionary = true;
                        continue;
                    }
                    session.set_zenz_candidates(draft);
                    return Ok(());
                }
                constraint = next;
            }
        }
    }

    unreachable!("the bounded inference loop always returns")
}

fn candidate_satisfies(candidate: &Candidate, constraint: &PrefixConstraint) -> bool {
    if constraint.has_eos {
        candidate.text.as_bytes() == constraint.bytes
    } else {
        candidate.text.as_bytes().starts_with(&constraint.bytes)
    }
}

fn candidate_uses_personal_dictionary(candidate: &Candidate) -> bool {
    candidate.entries.iter().any(|entry| {
        entry.metadata.contains(DictionaryMetadata::LEARNED)
            || entry.metadata.contains(DictionaryMetadata::USER_DICTIONARY)
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use beankey_converter::{
        ConversionSession, DictionaryStore, InputStyle, InputTableRegistry, NormalConverter,
        ZenzEvaluator, ZenzInferenceError, ZenzLanguageModel, ZenzV3Config, ZenzVersionConfig,
    };

    use super::{ZenzConversionOptions, convert};

    struct PrefixModel {
        evaluations: usize,
        prompts: Vec<String>,
    }

    impl ZenzLanguageModel for PrefixModel {
        fn vocabulary_size(&self) -> usize {
            5
        }

        fn eos_token(&self) -> i32 {
            2
        }

        fn tokenize(
            &mut self,
            text: &str,
            add_special: bool,
        ) -> Result<Vec<i32>, ZenzInferenceError> {
            if add_special {
                self.prompts.push(text.to_owned());
            }
            Ok(if add_special {
                vec![1]
            } else if text == "箸" {
                vec![4]
            } else {
                vec![3]
            })
        }

        fn token_to_piece(&mut self, token: i32) -> Result<Vec<u8>, ZenzInferenceError> {
            Ok(match token {
                4 => "箸".as_bytes().to_vec(),
                _ => b"x".to_vec(),
            })
        }

        fn next_logits(&mut self, _tokens: &[i32]) -> Result<Vec<f32>, ZenzInferenceError> {
            self.evaluations += 1;
            Ok(vec![0.0, 0.0, 0.0, 1.0, 10.0])
        }
    }

    fn dictionary_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/azooKey_dictionary_storage/Dictionary")
    }

    fn session() -> (DictionaryStore, InputTableRegistry, ConversionSession) {
        let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
        let tables = InputTableRegistry::new();
        let mut session = ConversionSession::new();
        session.insert_str("はし", InputStyle::Direct, &tables);
        (dictionary, tables, session)
    }

    #[test]
    fn constrains_a_retried_draft_to_the_model_prefix() {
        let (dictionary, tables, mut session) = session();
        let converter = NormalConverter::new(&dictionary);
        let mut model = PrefixModel {
            evaluations: 0,
            prompts: Vec::new(),
        };
        let mut evaluator = ZenzEvaluator::default();

        convert(
            &mut session,
            &converter,
            &tables,
            &mut model,
            &mut evaluator,
            ZenzConversionOptions {
                version: &ZenzVersionConfig::default(),
                request_rich_candidates: false,
                inference_limit: 10,
                personalization: None,
            },
        )
        .unwrap();

        assert_eq!(session.candidates()[0].text, "箸");
        assert!(model.evaluations >= 2);
    }

    #[test]
    fn returns_the_initial_draft_when_the_inference_limit_is_zero() {
        let (dictionary, tables, mut session) = session();
        let converter = NormalConverter::new(&dictionary);
        let mut model = PrefixModel {
            evaluations: 0,
            prompts: Vec::new(),
        };
        let mut evaluator = ZenzEvaluator::default();

        convert(
            &mut session,
            &converter,
            &tables,
            &mut model,
            &mut evaluator,
            ZenzConversionOptions {
                version: &ZenzVersionConfig::default(),
                request_rich_candidates: false,
                inference_limit: 0,
                personalization: None,
            },
        )
        .unwrap();

        assert!(!session.candidates().is_empty());
        assert_eq!(model.evaluations, 0);
    }

    #[test]
    fn includes_the_internal_composition_cursor_in_the_zenz_prompt() {
        let (dictionary, tables, mut session) = session();
        session.move_cursor(-1);
        let converter = NormalConverter::new(&dictionary);
        let mut model = PrefixModel {
            evaluations: 0,
            prompts: Vec::new(),
        };
        let mut evaluator = ZenzEvaluator::default();

        convert(
            &mut session,
            &converter,
            &tables,
            &mut model,
            &mut evaluator,
            ZenzConversionOptions {
                version: &ZenzVersionConfig::V3(ZenzV3Config {
                    enable_alignment_separator: true,
                    ..Default::default()
                }),
                request_rich_candidates: false,
                inference_limit: 1,
                personalization: None,
            },
        )
        .unwrap();

        assert!(
            model
                .prompts
                .iter()
                .any(|prompt| prompt.contains("\u{ee00}ハ\u{ee08}シ\u{ee01}"))
        );
    }
}
