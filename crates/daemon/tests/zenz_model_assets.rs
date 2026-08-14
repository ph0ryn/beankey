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
        session_id: "fixed-model".into(),
        payload: Some(payload),
        trace: Vec::new(),
    }
}

#[test]
fn converts_with_the_fixed_zenz_model_and_llama_backend() {
    let (Ok(model_path), Ok(backend_directory)) = (
        std::env::var("BEANKEY_TEST_MODEL"),
        std::env::var("BEANKEY_TEST_LLAMA_BACKEND"),
    ) else {
        return;
    };
    let mut engine = Engine::open_with_llama(dictionary_root(), model_path, backend_directory)
        .expect("the fixed model and pinned llama.cpp backend must load");
    engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
        }),
    ));

    let converted = engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            key_sym: 0,
            modifiers: 0,
            release: false,
            text: "はし".into(),
            surrounding_text: None,
            input: String::new(),
            intention: String::new(),
        }),
    ));

    let Some(Payload::StateResponse(state)) = converted.payload else {
        panic!("fixed-model conversion returned a protocol error");
    };
    assert_eq!(state.preedit, "はし");
    assert!(!state.candidates.is_empty());
}
