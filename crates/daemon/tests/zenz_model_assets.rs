use std::fs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use beankey_daemon::protocol::envelope::Payload;
use beankey_daemon::{Engine, PROTOCOL_VERSION, protocol, read_envelope, write_envelope};
use beankey_llama::{LlamaContext, LlamaSequence};
use tempfile::TempDir;

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
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    ));

    let first = engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            action: protocol::UserAction::Input as i32,
            shift: false,
            text: "は".into(),
            surrounding_text: None,
            input: String::new(),
            intention: String::new(),
            option: false,
        }),
    ));
    let Some(Payload::StateResponse(first)) = first.payload else {
        panic!("fixed-model conversion returned a protocol error");
    };
    assert_eq!(first.preedit, "は");

    let converted = engine.handle(envelope(
        3,
        Payload::KeyEvent(protocol::KeyEvent {
            action: protocol::UserAction::Input as i32,
            shift: false,
            text: "し".into(),
            surrounding_text: None,
            input: String::new(),
            intention: String::new(),
            option: false,
        }),
    ));

    let Some(Payload::StateResponse(state)) = converted.payload else {
        panic!("fixed-model conversion returned a protocol error");
    };
    assert_eq!(state.preedit, "はし");
    assert!(!state.candidates.is_empty());
}

#[test]
fn cached_and_batched_logits_match_a_single_full_evaluation() {
    let (Ok(model_path), Ok(backend_directory)) = (
        std::env::var("BEANKEY_TEST_MODEL"),
        std::env::var("BEANKEY_TEST_LLAMA_BACKEND"),
    ) else {
        return;
    };
    let mut context = LlamaContext::load(model_path, backend_directory).unwrap();
    let tokens = context
        .tokenize("\u{ee00}テスト\u{ee01}候補", true)
        .unwrap();
    assert!(tokens.len() >= 2);

    let batched = context
        .logits(&tokens, 0, LlamaSequence::Evaluation)
        .unwrap();
    let single = context.next_logits(&tokens).unwrap();
    let final_row = &batched[batched.len() - single.len()..];
    let maximum_difference = final_row
        .iter()
        .zip(&single)
        .map(|(batched, single)| (batched - single).abs())
        .fold(0.0_f32, f32::max);

    assert!(
        maximum_difference < 1e-5,
        "cached logits differed by {maximum_difference}"
    );
}

#[test]
fn runs_the_server_executable_with_the_fixed_nixos_assets() {
    let (Ok(model), Ok(backend), Ok(english), Ok(greek)) = (
        std::env::var("BEANKEY_TEST_MODEL"),
        std::env::var("BEANKEY_TEST_LLAMA_BACKEND"),
        std::env::var("BEANKEY_TEST_EN_US_DICTIONARY"),
        std::env::var("BEANKEY_TEST_EL_GR_DICTIONARY"),
    ) else {
        return;
    };
    let runtime = TempDir::new().unwrap();
    let config_path = runtime.path().join("config.toml");
    fs::write(
        &config_path,
        format!(
            r#"
dictionary = "{}"
model = "{model}"
emoji_dictionary = "{}"
llama_backend_directory = "{backend}"
runtime_socket = "beankey/daemon.sock"

[hunspell]
english_dictionary = "{english}"
greek_dictionary = "{greek}"

[conversion]
input_style = "roman_to_kana"
n_best = 10
japanese_prediction = "automatic"
foreign_prediction = "automatic"
full_width_roman = false
half_width_kana = false
typography = false
typo_correction = "automatic"
live_conversion = false
custom_input_tables = {{}}

[zenz]
inference_limit = 10
rich_candidates = false
predictive_input = false
enable_alignment_separator = false

[lm_typo]
enabled = false
language_model = "zenz"
beam_size = 32
top_k = 64
n_best = 5
substitution_cost = 2.0
deletion_cost = 3.0
transposition_cost = 2.0

[inference]
context_size = 512
batch_size = 512
micro_batch_size = 64
flash_attention = true
"#,
            dictionary_root().display(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("data/azooKey_emoji_dictionary_storage/EmojiDictionary/emoji_all_E17.0.txt")
                .display()
        ),
    )
    .unwrap();
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_beankey-daemon"))
        .args(["--config", config_path.to_str().unwrap()])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("XDG_STATE_HOME", runtime.path().join("state"))
        .spawn()
        .unwrap();
    let socket = runtime.path().join("beankey/daemon.sock");
    let mut stream = connect_before(&socket, Duration::from_secs(5));

    for request in [
        envelope(
            1,
            Payload::StartSession(protocol::StartSession {
                input_style: protocol::InputStyle::Direct as i32,
                surrounding_text: None,
                keyboard_language: protocol::KeyboardLanguage::EnglishUs as i32,
                custom_input_table: String::new(),
            }),
        ),
        envelope(
            2,
            Payload::KeyEvent(protocol::KeyEvent {
                action: protocol::UserAction::Input as i32,
                text: "hel".into(),
                ..Default::default()
            }),
        ),
    ] {
        write_envelope(&mut stream, &request).unwrap();
        let response = read_envelope(&mut stream).unwrap();
        let Some(Payload::StateResponse(state)) = response.payload else {
            panic!("server returned a protocol error");
        };
        if request.request_id == 2 {
            assert!(
                state
                    .candidates
                    .iter()
                    .any(|candidate| candidate.text.len() > 3 && candidate.text.starts_with("hel"))
            );
        }
    }
    drop(stream);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            daemon.kill().unwrap();
            panic!("daemon did not exit after its last client disconnected");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn connect_before(path: &Path, timeout: Duration) -> UnixStream {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("could not connect to daemon: {error}"),
        }
    }
}
