use std::path::PathBuf;

use beankey_daemon::protocol::envelope::Payload;
use beankey_daemon::{Engine, PROTOCOL_VERSION, protocol};

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

fn envelope(request_id: u64, payload: Payload) -> protocol::Envelope {
    protocol::Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        session_id: "session".into(),
        payload: Some(payload),
        trace: Vec::new(),
    }
}

fn response(envelope: protocol::Envelope) -> protocol::StateResponse {
    match envelope.payload.unwrap() {
        Payload::StateResponse(response) => response,
        Payload::ProtocolError(error) => panic!("protocol error: {}", error.message),
        _ => panic!("unexpected response"),
    }
}

#[test]
fn converts_selects_commits_and_resets_a_session() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
        }),
    )));
    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            key_sym: 0,
            modifiers: 0,
            release: false,
            text: "shikai".into(),
            surrounding_text: None,
            input: String::new(),
            intention: String::new(),
        }),
    )));
    assert!(converted.consumed);
    assert_eq!(converted.preedit, "しかい");
    let index = converted
        .candidates
        .iter()
        .position(|candidate| candidate.text == "司会")
        .unwrap();

    let selected = response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate {
            index: index as u32,
        }),
    )));
    assert_eq!(selected.commit, "司会");
    assert!(selected.preedit.is_empty());
    assert!(selected.reset);

    let reset = response(engine.handle(envelope(
        4,
        Payload::ResetSession(protocol::ResetSession {}),
    )));
    assert!(reset.reset);
    assert!(reset.preedit.is_empty());
}

#[test]
fn forwards_explicit_kana_key_intention_and_input() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::KanaJis as i32,
            surrounding_text: None,
        }),
    )));
    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            key_sym: 0,
            modifiers: 0,
            release: false,
            text: "q".into(),
            surrounding_text: None,
            input: "q".into(),
            intention: "q".into(),
        }),
    )));
    assert!(converted.consumed);
    assert_eq!(converted.preedit, "た");
}

#[test]
fn isolates_sessions_and_rejects_out_of_order_requests() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        2,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
        }),
    )));
    let error = engine.handle(envelope(
        1,
        Payload::ResetSession(protocol::ResetSession {}),
    ));
    assert!(matches!(error.payload, Some(Payload::ProtocolError(_))));

    let mut other = envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
        }),
    );
    other.session_id = "other".into();
    response(engine.handle(other));
}
