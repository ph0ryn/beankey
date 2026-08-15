use std::collections::BTreeMap;
use std::path::PathBuf;

use beankey_converter::{ZenzInferenceError, ZenzLanguageModel};
use beankey_daemon::protocol::envelope::Payload;
use beankey_daemon::{
    ConversionConfig, ConversionResourceError, Engine, EngineOpenError, PROTOCOL_VERSION, protocol,
};
use tempfile::TempDir;

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

fn emoji_dictionary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_emoji_dictionary_storage/EmojiDictionary/emoji_all_E17.0.txt")
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

fn start_roman_session(engine: &mut Engine, request_id: u64) {
    response(engine.handle(envelope(
        request_id,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
}

fn key_event(key_sym: u32, text: &str) -> protocol::KeyEvent {
    protocol::KeyEvent {
        key_sym,
        text: text.into(),
        input: text.into(),
        intention: text.into(),
        ..Default::default()
    }
}

#[test]
fn does_not_turn_control_key_text_into_composition() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);

    for (index, (name, key_sym, text)) in [
        ("BackSpace", 0xff08, "\u{8}"),
        ("Return", 0xff0d, "\r"),
        ("KP_Enter", 0xff8d, "\r"),
        ("Escape", 0xff1b, "\u{1b}"),
        ("Tab", 0xff09, "\t"),
        ("Delete", 0xffff, "\u{7f}"),
    ]
    .into_iter()
    .enumerate()
    {
        let request_id = index as u64 * 2 + 2;
        let state = response(engine.handle(envelope(
            request_id,
            Payload::KeyEvent(protocol::KeyEvent {
                key_sym,
                text: text.into(),
                input: text.into(),
                intention: text.into(),
                ..Default::default()
            }),
        )));
        assert!(!state.consumed, "{name} must be returned to Fcitx5");
        assert!(state.preedit.is_empty(), "{name} created preedit text");
        assert!(state.commit.is_empty(), "{name} committed control text");

        let reset = response(engine.handle(envelope(
            request_id + 1,
            Payload::ResetSession(protocol::ResetSession {}),
        )));
        assert!(reset.reset);
    }
}

#[test]
fn edits_active_composition_with_backspace_and_delete() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);

    let composing = response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "kana")))));
    assert_eq!(composing.preedit, "かな");

    let backspace =
        response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff08, "\u{8}")))));
    assert!(backspace.consumed);
    assert_eq!(backspace.preedit, "か");
    assert!(backspace.commit.is_empty());

    response(engine.handle(envelope(
        4,
        Payload::ResetSession(protocol::ResetSession {}),
    )));
    response(engine.handle(envelope(5, Payload::KeyEvent(key_event(0, "kana")))));
    let moved = response(engine.handle(envelope(6, Payload::KeyEvent(key_event(0xff51, "")))));
    assert_eq!(moved.preedit_cursor, 1);

    let delete =
        response(engine.handle(envelope(7, Payload::KeyEvent(key_event(0xffff, "\u{7f}")))));
    assert!(delete.consumed);
    assert_eq!(delete.preedit, "か");
    assert_eq!(delete.preedit_cursor, 1);
    assert!(delete.commit.is_empty());
}

#[test]
fn commits_active_composition_with_return_and_keypad_enter() {
    for (name, key_sym) in [("Return", 0xff0d), ("KP_Enter", 0xff8d)] {
        let mut engine = Engine::open(dictionary_root()).unwrap();
        start_roman_session(&mut engine, 1);
        response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "shikai")))));

        let committed =
            response(engine.handle(envelope(3, Payload::KeyEvent(key_event(key_sym, "\r")))));
        assert!(committed.consumed, "{name} did not commit the composition");
        assert!(
            !committed.commit.is_empty(),
            "{name} returned an empty commit"
        );
        assert!(
            !committed.commit.chars().any(char::is_control),
            "{name} committed control text"
        );
        assert!(committed.preedit.is_empty());
    }
}

#[test]
fn escape_cancels_active_composition() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);
    response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "kana")))));

    let escaped =
        response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff1b, "\u{1b}")))));
    assert!(escaped.consumed);
    assert!(escaped.reset);
    assert!(escaped.preedit.is_empty());
    assert!(escaped.candidates.is_empty());
    assert!(escaped.commit.is_empty());
}

#[test]
fn returns_tab_without_mutating_active_composition() {
    for (name, key_sym) in [("Tab", 0xff09), ("ISO_Left_Tab", 0xfe20)] {
        let mut engine = Engine::open(dictionary_root()).unwrap();
        start_roman_session(&mut engine, 1);
        let composing =
            response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "kana")))));

        let tab = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(key_sym, "\t")))));
        assert!(!tab.consumed, "{name} must be returned to Fcitx5");
        assert_eq!(tab.preedit, composing.preedit, "{name} changed preedit");
        assert_eq!(
            tab.preedit_cursor, composing.preedit_cursor,
            "{name} moved the preedit cursor"
        );
        assert!(tab.commit.is_empty(), "{name} committed text");
    }
}

#[test]
fn returns_enter_after_committing_a_candidate() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);
    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "shikai".into(),
            ..Default::default()
        }),
    )));
    let index = converted
        .candidates
        .iter()
        .position(|candidate| candidate.text == "司会")
        .unwrap();
    let committed = response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate {
            index: index as u32,
        }),
    )));
    assert_eq!(committed.commit, "司会");
    assert!(!committed.candidates.is_empty());

    let enter = response(engine.handle(envelope(
        4,
        Payload::KeyEvent(protocol::KeyEvent {
            key_sym: 0xff0d,
            ..Default::default()
        }),
    )));
    assert!(
        !enter.consumed,
        "Enter selected a post-composition prediction"
    );
    assert!(enter.commit.is_empty());
    assert!(enter.preedit.is_empty());
}

#[test]
fn does_not_persist_control_key_text_in_learning_memory() {
    for (name, key_sym, text) in [
        ("BackSpace", 0xff08, "\u{8}"),
        ("Return", 0xff0d, "\r"),
        ("KP_Enter", 0xff8d, "\r"),
        ("Escape", 0xff1b, "\u{1b}"),
        ("Tab", 0xff09, "\t"),
        ("ISO_Left_Tab", 0xfe20, "\t"),
        ("Delete", 0xffff, "\u{7f}"),
    ] {
        let state = TempDir::new().unwrap();
        let mut engine = Engine::open_with_learning(dictionary_root(), state.path()).unwrap();
        start_roman_session(&mut engine, 1);
        response(engine.handle(envelope(2, Payload::KeyEvent(key_event(key_sym, text)))));
        response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff0d, "")))));

        assert!(
            !state.path().join("memory.bin").exists(),
            "{name} control text reached persistent learning memory"
        );
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
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
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
    assert!(!selected.reset);
    assert!(!selected.candidates.is_empty());

    let reset = response(engine.handle(envelope(
        4,
        Payload::ResetSession(protocol::ResetSession {}),
    )));
    assert!(reset.reset);
    assert!(reset.preedit.is_empty());
}

#[test]
fn uses_the_configured_input_style_when_the_addon_does_not_override_it() {
    let mut engine =
        Engine::open_with_conversion_resources(dictionary_root(), &ConversionConfig::default())
            .unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Unspecified as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));

    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "shikai".into(),
            ..Default::default()
        }),
    )));

    assert_eq!(converted.preedit, "しかい");
    assert!(
        converted
            .candidates
            .iter()
            .any(|candidate| candidate.text == "司会")
    );
}

#[test]
fn forwards_explicit_kana_key_intention_and_input() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::KanaJis as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
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
fn completes_foreign_input_with_the_configured_hunspell_assets() {
    let mut engine = Engine::open_with_hunspell(
        dictionary_root(),
        env!("BEANKEY_TEST_EN_US_DICTIONARY"),
        env!("BEANKEY_TEST_EL_GR_DICTIONARY"),
    )
    .unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::EnglishUs as i32,
            custom_input_table: String::new(),
        }),
    )));
    let english = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "hel".into(),
            ..Default::default()
        }),
    )));
    assert!(
        english
            .candidates
            .iter()
            .any(|candidate| candidate.text.len() > 3 && candidate.text.starts_with("hel"))
    );

    response(engine.handle(envelope(
        3,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Greek as i32,
            custom_input_table: String::new(),
        }),
    )));
    let greek = response(engine.handle(envelope(
        4,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "καλ".into(),
            ..Default::default()
        }),
    )));
    assert!(
        greek
            .candidates
            .iter()
            .any(|candidate| candidate.text == "καλά")
    );
}

#[test]
fn persists_forgets_and_resets_learning_through_session_requests() {
    let state = TempDir::new().unwrap();
    let mut engine = Engine::open_with_learning(dictionary_root(), state.path()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "shikai".into(),
            ..Default::default()
        }),
    )));
    let selected = converted
        .candidates
        .iter()
        .position(|candidate| candidate.text == "司会")
        .unwrap();
    response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate {
            index: selected as u32,
        }),
    )));
    let memory_path = state.path().join("memory.bin");
    let learned_size = std::fs::metadata(&memory_path).unwrap().len();
    drop(engine);

    let mut restarted = Engine::open_with_learning(dictionary_root(), state.path()).unwrap();
    response(restarted.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    let converted = response(restarted.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "shikai".into(),
            ..Default::default()
        }),
    )));
    let learned = converted
        .candidates
        .iter()
        .position(|candidate| candidate.text == "司会")
        .unwrap();
    response(restarted.handle(envelope(
        3,
        Payload::ForgetCandidate(protocol::ForgetCandidate {
            index: learned as u32,
        }),
    )));
    assert!(std::fs::metadata(&memory_path).unwrap().len() < learned_size);

    response(restarted.handle(envelope(
        4,
        Payload::ResetLearning(protocol::ResetLearning {}),
    )));
    assert!(!memory_path.exists());
}

#[test]
fn offers_and_commits_emoji_post_composition_predictions() {
    let mut engine = Engine::open_with_emoji(dictionary_root(), emoji_dictionary_path()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "egao".into(),
            ..Default::default()
        }),
    )));
    let smile = converted
        .candidates
        .iter()
        .position(|candidate| candidate.text == "笑顔")
        .unwrap();
    let committed = response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate {
            index: smile as u32,
        }),
    )));
    assert_eq!(committed.commit, "笑顔");
    assert!(committed.preedit.is_empty());
    assert!(!committed.reset);
    assert_eq!(
        committed.candidates[..3]
            .iter()
            .filter(|candidate| candidate.value == -3.0)
            .count(),
        3
    );

    let emoji = committed
        .candidates
        .iter()
        .position(|candidate| candidate.value == -3.0)
        .unwrap();
    let selected = response(engine.handle(envelope(
        4,
        Payload::SelectCandidate(protocol::SelectCandidate {
            index: emoji as u32,
        }),
    )));
    assert!(!selected.commit.is_empty());
    assert!(!selected.commit.is_ascii());
}

#[test]
fn uses_a_json_user_dictionary_with_a_named_custom_input_table() {
    let resources = TempDir::new().unwrap();
    let user_dictionary = resources.path().join("user.json");
    let input_table = resources.path().join("greeting.tsv");
    std::fs::write(
        &user_dictionary,
        r#"[{"word":"挨拶語","reading":"あいさつ","hint":"test"}]"#,
    )
    .unwrap();
    std::fs::write(&input_table, "qq\tあいさつ\n").unwrap();
    let mut engine = Engine::open_with_conversion_resources(
        dictionary_root(),
        &ConversionConfig {
            user_dictionary: Some(user_dictionary),
            user_dictionary_directory: None,
            custom_input_tables: BTreeMap::from([("greeting".into(), input_table)]),
            ..Default::default()
        },
    )
    .unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Custom as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: "greeting".into(),
        }),
    )));

    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "qq".into(),
            ..Default::default()
        }),
    )));

    assert_eq!(converted.preedit, "あいさつ");
    assert!(
        converted
            .candidates
            .iter()
            .any(|candidate| candidate.text == "挨拶語")
    );
}

#[test]
fn applies_conversion_options_and_displays_live_conversion() {
    let mut engine = Engine::open_with_conversion_resources(
        dictionary_root(),
        &ConversionConfig {
            n_best: 3,
            live_conversion: true,
            full_width_roman: true,
            typography: true,
            ..Default::default()
        },
    )
    .unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));

    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "かなかんじへんかん".into(),
            ..Default::default()
        }),
    )));

    assert_ne!(converted.preedit, "かなかんじへんかん");
    assert_eq!(
        converted.preedit_cursor as usize,
        converted.preedit.chars().count()
    );
    assert!(converted.candidates.len() >= 3);
    let committed = response(engine.handle(envelope(
        3,
        Payload::KeyEvent(protocol::KeyEvent {
            key_sym: 0xff0d,
            ..Default::default()
        }),
    )));
    assert_eq!(committed.commit, converted.preedit);

    response(engine.handle(envelope(
        4,
        Payload::ResetSession(protocol::ResetSession {}),
    )));
    let representations = response(engine.handle(envelope(
        5,
        Payload::KeyEvent(protocol::KeyEvent {
            text: "ABC".into(),
            ..Default::default()
        }),
    )));
    assert!(
        representations
            .candidates
            .iter()
            .any(|candidate| candidate.text == "ＡＢＣ")
    );
}

#[test]
fn rejects_an_invalid_custom_input_table() {
    let resources = TempDir::new().unwrap();
    let input_table = resources.path().join("invalid.tsv");
    std::fs::write(&input_table, "missing separator\n").unwrap();

    let error = match Engine::open_with_conversion_resources(
        dictionary_root(),
        &ConversionConfig {
            custom_input_tables: BTreeMap::from([("invalid".into(), input_table)]),
            ..Default::default()
        },
    ) {
        Ok(_) => panic!("invalid input table was accepted"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        EngineOpenError::ConversionResource(ConversionResourceError::InvalidInputTable {
            name
        }) if name == "invalid"
    ));
}

#[test]
fn isolates_sessions_and_rejects_out_of_order_requests() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        2,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
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
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    );
    other.session_id = "other".into();
    response(engine.handle(other));
}

#[test]
fn rejects_invalid_surrounding_text_and_resets_the_session() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    let rejected = engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: Some(protocol::SurroundingText {
                available: true,
                text: "文脈".into(),
                cursor: 3,
                anchor: 0,
            }),
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    ));

    let Some(Payload::ProtocolError(error)) = rejected.payload else {
        panic!("expected a protocol error");
    };
    assert_eq!(
        error.code,
        protocol::protocol_error::Code::InvalidPayload as i32
    );
}

struct PrefixModel;

impl ZenzLanguageModel for PrefixModel {
    fn vocabulary_size(&self) -> usize {
        5
    }

    fn eos_token(&self) -> i32 {
        2
    }

    fn tokenize(&mut self, text: &str, add_special: bool) -> Result<Vec<i32>, ZenzInferenceError> {
        Ok(if add_special {
            vec![1]
        } else if text == "箸" {
            vec![4]
        } else {
            vec![3]
        })
    }

    fn token_to_piece(&mut self, token: i32) -> Result<Vec<u8>, ZenzInferenceError> {
        Ok(if token == 4 {
            "箸".as_bytes().to_vec()
        } else {
            b"x".to_vec()
        })
    }

    fn next_logits(&mut self, _tokens: &[i32]) -> Result<Vec<f32>, ZenzInferenceError> {
        Ok(vec![0.0, 0.0, 0.0, 1.0, 10.0])
    }
}

#[test]
fn applies_zenz_prefix_correction_through_the_session_engine() {
    let mut engine =
        Engine::open_with_zenz_model(dictionary_root(), Box::new(PrefixModel)).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));

    let converted = response(engine.handle(envelope(
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
    )));

    assert!(
        converted
            .candidates
            .iter()
            .take(5)
            .any(|candidate| candidate.text == "箸")
    );
}

#[test]
fn preserves_special_candidates_after_zenz_conversion() {
    let mut engine =
        Engine::open_with_zenz_model(dictionary_root(), Box::new(PrefixModel)).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));

    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            key_sym: 0,
            modifiers: 0,
            release: false,
            text: "U+1F600".into(),
            surrounding_text: None,
            input: String::new(),
            intention: String::new(),
        }),
    )));

    assert!(
        converted
            .candidates
            .iter()
            .any(|candidate| candidate.text == "😀")
    );
}
