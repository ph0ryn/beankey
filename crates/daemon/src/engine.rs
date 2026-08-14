use std::collections::HashMap;
use std::path::Path;

use beankey_converter::{
    CompleteAction, ComposingCount as ConverterComposingCount, ConversionSession, DictionaryError,
    DictionaryStore, InputModifier, InputStyle as ConverterInputStyle, InputTableId,
    InputTableRegistry, NormalConverter, RequestOptions,
};

use crate::protocol::composing_count::Count;
use crate::protocol::envelope::Payload;
use crate::protocol::protocol_error::Code;
use crate::{PROTOCOL_VERSION, protocol};

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
}

pub struct Engine {
    dictionary: DictionaryStore,
    tables: InputTableRegistry,
    sessions: HashMap<String, SessionState>,
}

impl Engine {
    pub fn open(dictionary_path: impl AsRef<Path>) -> Result<Self, DictionaryError> {
        Ok(Self {
            dictionary: DictionaryStore::open(dictionary_path.as_ref().to_path_buf())?,
            tables: InputTableRegistry::new(),
            sessions: HashMap::new(),
        })
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
                self.sessions.insert(
                    session_id.clone(),
                    SessionState {
                        conversion: ConversionSession::new(),
                        input_style,
                        last_request_id: request_id,
                        selected_candidate: 0,
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
            Payload::KeyEvent(event) => self.handle_key(&mut session, event),
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
            | Payload::ProtocolError(_) => Err("invalid session request"),
        };
        self.sessions.insert(session_id.clone(), session);
        match response {
            Ok(response) => state_envelope(request_id, session_id, response),
            Err(message) => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.conversion.reset();
                    session.selected_candidate = 0;
                }
                error_envelope(request_id, session_id, Code::Internal, message)
            }
        }
    }

    fn handle_key(
        &self,
        session: &mut SessionState,
        event: protocol::KeyEvent,
    ) -> Result<protocol::StateResponse, &'static str> {
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
            let converter = NormalConverter::new(&self.dictionary);
            session
                .conversion
                .request(&converter, &self.tables, RequestOptions::default())
                .map_err(|_| "candidate generation failed")?;
        }
        Ok(make_state(session, true, String::new(), false))
    }

    fn select_candidate(
        &self,
        session: &mut SessionState,
        index: usize,
    ) -> Result<protocol::StateResponse, &'static str> {
        let commit = session
            .conversion
            .select_candidate(index, &self.tables)
            .map_err(|_| "candidate selection failed")?;
        session.selected_candidate = 0;
        if !session.conversion.composing().is_empty() {
            let converter = NormalConverter::new(&self.dictionary);
            session
                .conversion
                .request(&converter, &self.tables, RequestOptions::default())
                .map_err(|_| "candidate generation failed")?;
        }
        Ok(make_state(
            session,
            true,
            commit,
            session.conversion.composing().is_empty(),
        ))
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
