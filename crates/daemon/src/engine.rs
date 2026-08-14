use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use beankey_converter::{
    CompleteAction, ComposingCount as ConverterComposingCount, ConversionSession, DictionaryError,
    DictionaryStore, InputModifier, InputStyle as ConverterInputStyle, InputTableId,
    InputTableRegistry, NormalConverter, RequestOptions, SelectionError, ZenzLanguageModel,
    ZenzVersionConfig,
};
use beankey_llama::LlamaError;

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
}

#[derive(Debug)]
pub enum EngineOpenError {
    Dictionary(DictionaryError),
    Llama(LlamaError),
}

impl fmt::Display for EngineOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dictionary(error) => error.fmt(formatter),
            Self::Llama(error) => error.fmt(formatter),
        }
    }
}

impl Error for EngineOpenError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Dictionary(error) => Some(error),
            Self::Llama(error) => Some(error),
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
                let Some(input_style) = input_style(start.input_style) else {
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
                self.sessions.insert(
                    session_id.clone(),
                    SessionState {
                        conversion: ConversionSession::new(),
                        input_style,
                        last_request_id: request_id,
                        selected_candidate: 0,
                        surrounding,
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
            KEY_ESCAPE if !session.conversion.composing().is_empty() => {
                session.conversion.reset();
                session.selected_candidate = 0;
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
            KEY_SPACE if !session.conversion.composing().is_empty() => {
                if !session.conversion.candidates().is_empty() {
                    session.selected_candidate =
                        (session.selected_candidate + 1) % session.conversion.candidates().len();
                    return Ok(make_state(session, true, String::new(), false));
                }
            }
            KEY_UP | KEY_DOWN if !session.conversion.candidates().is_empty() => {
                let count = session.conversion.candidates().len();
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
                if matches!(session.input_style, ConverterInputStyle::Mapped(_)) {
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
        session.selected_candidate = 0;
        if !session.conversion.composing().is_empty() {
            self.request_candidates(session)?;
        }
        Ok(make_state(
            session,
            true,
            commit,
            session.conversion.composing().is_empty(),
        ))
    }

    fn request_candidates(&mut self, session: &mut SessionState) -> SessionRequestResult<()> {
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
                .finalize_zenz_request(&converter, &self.tables, RequestOptions::default())
                .map_err(|error| {
                    (
                        Code::Internal,
                        format!("candidate assembly failed: {error}"),
                    )
                })?;
        } else {
            session
                .conversion
                .request(&converter, &self.tables, RequestOptions::default())
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

fn input_style(value: i32) -> Option<ConverterInputStyle> {
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
    }
}

fn page_candidates(session: &mut SessionState, page: protocol::PageCandidates) -> bool {
    let count = session.conversion.candidates().len();
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
        candidates: session
            .conversion
            .candidates()
            .iter()
            .map(candidate_to_protocol)
            .collect(),
        selected_candidate: session.selected_candidate as i32,
        commit,
        reset,
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
