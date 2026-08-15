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

#[test]
fn advertises_only_configured_addon_capabilities() {
    let mut stateless = Engine::open(dictionary_root()).unwrap();
    let stateless_start = response(stateless.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    assert!(!stateless_start.lm_typo_available);
    assert!(!stateless_start.learning_available);
    assert!(!stateless_start.learning_writable);

    let state = TempDir::new().unwrap();
    let mut learning = Engine::open_with_learning(dictionary_root(), state.path()).unwrap();
    let learning_start = response(learning.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::RomanToKana as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    assert!(!learning_start.lm_typo_available);
    assert!(learning_start.learning_available);
    assert!(learning_start.learning_writable);
}

fn key_event(key_sym: u32, text: &str) -> protocol::KeyEvent {
    protocol::KeyEvent {
        action: match key_sym {
            0 => protocol::UserAction::Input,
            0xff08 => protocol::UserAction::Backspace,
            0xffff => protocol::UserAction::DeleteForward,
            0xff0d | 0xff8d => protocol::UserAction::Enter,
            0xff1b => protocol::UserAction::Escape,
            0xff09 | 0xfe20 => protocol::UserAction::Tab,
            0xff51 => protocol::UserAction::Left,
            0xff52 => protocol::UserAction::Up,
            0xff53 => protocol::UserAction::Right,
            0xff54 => protocol::UserAction::Down,
            0x20 => protocol::UserAction::Space,
            _ => protocol::UserAction::Unspecified,
        } as i32,
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
            Payload::KeyEvent(key_event(key_sym, text)),
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
fn edits_active_composition_with_backspace_and_consumes_navigation() {
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
    assert!(moved.consumed);
    assert_eq!(moved.preedit_cursor, 2);
    assert_eq!(moved.preedit, "かな");
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
fn consumes_tab_without_mutating_active_composition() {
    for (name, key_sym) in [("Tab", 0xff09), ("ISO_Left_Tab", 0xfe20)] {
        let mut engine = Engine::open(dictionary_root()).unwrap();
        start_roman_session(&mut engine, 1);
        let composing =
            response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "kana")))));

        let tab = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(key_sym, "\t")))));
        assert!(tab.consumed, "{name} must be consumed while composing");
        assert_eq!(tab.preedit, composing.preedit, "{name} changed preedit");
        assert_eq!(
            tab.preedit_cursor, composing.preedit_cursor,
            "{name} moved the preedit cursor"
        );
        assert!(tab.commit.is_empty(), "{name} committed text");
    }
}

#[test]
fn follows_the_non_live_desktop_candidate_state_transitions() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);

    let composing = response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "shikai")))));
    assert_eq!(
        composing.input_state,
        protocol::InputState::Composing as i32
    );
    assert_eq!(
        composing.candidate_window,
        protocol::CandidateWindow::Preview as i32
    );
    assert_eq!(composing.preedit, "しかい");

    let previewing = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0x20, "")))));
    assert_eq!(
        previewing.input_state,
        protocol::InputState::Previewing as i32
    );
    assert_eq!(
        previewing.candidate_window,
        protocol::CandidateWindow::Hidden as i32
    );
    assert_eq!(previewing.selected_candidate, -1);
    assert_ne!(previewing.preedit, "しかい");

    let selecting = response(engine.handle(envelope(4, Payload::KeyEvent(key_event(0x20, "")))));
    assert_eq!(
        selecting.input_state,
        protocol::InputState::Selecting as i32
    );
    assert_eq!(
        selecting.candidate_window,
        protocol::CandidateWindow::Selecting as i32
    );
    assert_eq!(selecting.selected_candidate, 0);

    let escaped = response(engine.handle(envelope(5, Payload::KeyEvent(key_event(0xff1b, "")))));
    assert_eq!(escaped.input_state, protocol::InputState::Previewing as i32);
    assert_eq!(
        escaped.candidate_window,
        protocol::CandidateWindow::Hidden as i32
    );

    let composing_again =
        response(engine.handle(envelope(6, Payload::KeyEvent(key_event(0xff1b, "")))));
    assert_eq!(
        composing_again.input_state,
        protocol::InputState::Composing as i32
    );
    assert_eq!(composing_again.preedit, "しかい");
}

#[test]
fn follows_live_desktop_selection_and_backspace_behavior() {
    let mut engine = Engine::open_with_conversion_resources(
        dictionary_root(),
        &ConversionConfig {
            live_conversion: true,
            ..Default::default()
        },
    )
    .unwrap();
    start_roman_session(&mut engine, 1);

    let composing = response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "shikai")))));
    assert_eq!(
        composing.candidate_window,
        protocol::CandidateWindow::Hidden as i32
    );
    assert_ne!(composing.preedit, "しかい");

    let selecting = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0x20, "")))));
    assert_eq!(selecting.selected_candidate, 0);
    assert_eq!(
        selecting.input_state,
        protocol::InputState::Selecting as i32
    );

    let next = response(engine.handle(envelope(4, Payload::KeyEvent(key_event(0x20, "")))));
    assert_eq!(next.selected_candidate, 1);
    let mut previous_space = key_event(0x20, "");
    previous_space.shift = true;
    let previous = response(engine.handle(envelope(5, Payload::KeyEvent(previous_space))));
    assert_eq!(previous.selected_candidate, 0);

    let raw = response(engine.handle(envelope(6, Payload::KeyEvent(key_event(0xff08, "")))));
    assert_eq!(raw.input_state, protocol::InputState::Composing as i32);
    assert_eq!(raw.preedit, "しか");
    assert_eq!(
        raw.candidate_window,
        protocol::CandidateWindow::Hidden as i32
    );
}

#[test]
fn zero_commits_selected_marked_text_and_starts_a_new_composition() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);
    response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "shikai")))));
    response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff54, "")))));

    let zero = response(engine.handle(envelope(4, Payload::KeyEvent(key_event(0, "0")))));
    assert!(zero.consumed);
    assert!(!zero.commit.is_empty());
    assert_eq!(zero.preedit, "0");
    assert_eq!(zero.input_state, protocol::InputState::Composing as i32);
}

#[test]
fn reveals_desktop_representation_candidates_progressively_above_the_first_candidate() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);
    response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "shikai")))));
    response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff54, "")))));

    let expected = [
        vec!["ひらがな"],
        vec!["カタカナ", "ひらがな"],
        vec!["半角カナ", "カタカナ", "ひらがな"],
        vec!["全角英数", "半角カナ", "カタカナ", "ひらがな"],
        vec!["英数", "全角英数", "半角カナ", "カタカナ", "ひらがな"],
    ];
    for (offset, annotations) in expected.into_iter().enumerate() {
        let state = response(engine.handle(envelope(
            4 + offset as u64,
            Payload::KeyEvent(key_event(0xff52, "")),
        )));
        assert_eq!(state.selected_candidate, 0);
        assert_eq!(
            state
                .candidates
                .iter()
                .take(annotations.len())
                .map(|candidate| candidate.annotation.as_str())
                .collect::<Vec<_>>(),
            annotations
        );
    }
}

#[test]
fn exposes_manual_prediction_separately_and_accepts_it_with_tab() {
    let mut engine = Engine::open_with_conversion_resources(
        dictionary_root(),
        &ConversionConfig {
            input_style: beankey_daemon::InputStyleConfig::Direct,
            japanese_prediction: beankey_daemon::PredictionConfig::Manual,
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

    let composing = response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "きょ")))));
    let prediction = composing
        .prediction
        .expect("manual prediction was not exposed");
    assert!(!prediction.display_text.is_empty());
    assert!(!prediction.append_text.is_empty());
    assert!(
        composing
            .candidates
            .iter()
            .all(|candidate| candidate.text != prediction.display_text)
    );

    let accepted = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff09, "")))));
    assert!(accepted.consumed);
    assert_ne!(accepted.preedit, composing.preedit);
}

#[test]
fn shift_navigation_edits_the_selected_segment_length() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "きょうはあめ")))));
    let initial = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff54, "")))));
    let initial_remaining =
        initial.preedit.chars().count() - initial.highlighted_preedit_length as usize;

    let mut shift_right = key_event(0xff53, "");
    shift_right.shift = true;
    let one = response(engine.handle(envelope(4, Payload::KeyEvent(shift_right.clone()))));
    assert!(one.highlighted_preedit_length > 0);
    let one_remaining = one.preedit.chars().count() - one.highlighted_preedit_length as usize;
    assert_eq!(one_remaining + 1, initial_remaining);

    let two = response(engine.handle(envelope(5, Payload::KeyEvent(shift_right))));
    let two_remaining = two.preedit.chars().count() - two.highlighted_preedit_length as usize;
    assert_eq!(two_remaining + 1, one_remaining);

    let mut shift_left = key_event(0xff51, "");
    shift_left.shift = true;
    let one_again = response(engine.handle(envelope(6, Payload::KeyEvent(shift_left))));
    let one_again_remaining =
        one_again.preedit.chars().count() - one_again.highlighted_preedit_length as usize;
    assert_eq!(one_again_remaining, one_remaining);
}

#[test]
fn transformed_candidate_commits_only_the_selected_segment() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    response(engine.handle(envelope(
        1,
        Payload::StartSession(protocol::StartSession {
            input_style: protocol::InputStyle::Direct as i32,
            surrounding_text: None,
            keyboard_language: protocol::KeyboardLanguage::Japanese as i32,
            custom_input_table: String::new(),
        }),
    )));
    response(engine.handle(envelope(2, Payload::KeyEvent(key_event(0, "きょうはあめ")))));
    let selecting = response(engine.handle(envelope(3, Payload::KeyEvent(key_event(0xff54, "")))));
    assert!(selecting.highlighted_preedit_length < selecting.preedit.chars().count() as u32);

    let transformed = response(engine.handle(envelope(
        4,
        Payload::KeyEvent(protocol::KeyEvent {
            action: protocol::UserAction::Hiragana as i32,
            ..Default::default()
        }),
    )));
    assert!(transformed.consumed);
    assert!(!transformed.commit.is_empty());
    assert!(!transformed.reset);
    assert!(!transformed.preedit.is_empty());
    assert_eq!(
        transformed.input_state,
        protocol::InputState::Selecting as i32
    );
}

#[test]
fn returns_enter_after_committing_a_candidate() {
    let mut engine = Engine::open(dictionary_root()).unwrap();
    start_roman_session(&mut engine, 1);
    let converted = response(engine.handle(envelope(
        2,
        Payload::KeyEvent(protocol::KeyEvent {
            action: protocol::UserAction::Input as i32,
            text: "shikai".into(),
            ..Default::default()
        }),
    )));
    let index = converted
        .candidates
        .iter()
        .find(|candidate| candidate.text == "司会")
        .unwrap()
        .index;
    let committed = response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate { index }),
    )));
    assert_eq!(committed.commit, "司会");
    assert!(committed.candidates.is_empty());

    let enter = response(engine.handle(envelope(
        4,
        Payload::KeyEvent(protocol::KeyEvent {
            action: protocol::UserAction::Enter as i32,
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
            action: protocol::UserAction::Input as i32,
            shift: false,
            text: "shikai".into(),
            surrounding_text: None,
            input: String::new(),
            intention: String::new(),
        }),
    )));
    assert!(converted.consumed);
    assert_eq!(converted.preedit, "しかい");
    assert_eq!(
        converted.candidate_window,
        protocol::CandidateWindow::Preview as i32
    );
    let index = converted
        .candidates
        .iter()
        .find(|candidate| candidate.text == "司会")
        .unwrap()
        .index;

    let selected = response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate { index }),
    )));
    assert_eq!(selected.commit, "司会");
    assert!(selected.preedit.is_empty());
    assert!(selected.reset);
    assert!(selected.candidates.is_empty());

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
            action: protocol::UserAction::Input as i32,
            text: "shikai".into(),
            ..Default::default()
        }),
    )));

    assert_ne!(converted.preedit, "shikai");
    assert_eq!(
        converted.candidate_window,
        protocol::CandidateWindow::Hidden as i32
    );
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
            action: protocol::UserAction::Input as i32,
            shift: false,
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
            action: protocol::UserAction::Input as i32,
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
            action: protocol::UserAction::Input as i32,
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
            action: protocol::UserAction::Input as i32,
            text: "shikai".into(),
            ..Default::default()
        }),
    )));
    let selected = converted
        .candidates
        .iter()
        .find(|candidate| candidate.text == "司会")
        .unwrap()
        .index;
    response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate { index: selected }),
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
            action: protocol::UserAction::Input as i32,
            text: "shikai".into(),
            ..Default::default()
        }),
    )));
    let learned = converted
        .candidates
        .iter()
        .find(|candidate| candidate.text == "司会")
        .unwrap()
        .index;
    response(restarted.handle(envelope(
        3,
        Payload::ForgetCandidate(protocol::ForgetCandidate { index: learned }),
    )));
    assert!(std::fs::metadata(&memory_path).unwrap().len() < learned_size);

    response(restarted.handle(envelope(
        4,
        Payload::ResetLearning(protocol::ResetLearning {}),
    )));
    assert!(!memory_path.exists());
}

#[test]
fn does_not_offer_post_composition_predictions_in_the_desktop_ux() {
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
            action: protocol::UserAction::Input as i32,
            text: "egao".into(),
            ..Default::default()
        }),
    )));
    let smile = converted
        .candidates
        .iter()
        .find(|candidate| candidate.text == "笑顔")
        .unwrap()
        .index;
    let committed = response(engine.handle(envelope(
        3,
        Payload::SelectCandidate(protocol::SelectCandidate { index: smile }),
    )));
    assert_eq!(committed.commit, "笑顔");
    assert!(committed.preedit.is_empty());
    assert!(committed.reset);
    assert!(committed.candidates.is_empty());
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
            action: protocol::UserAction::Input as i32,
            text: "qq".into(),
            ..Default::default()
        }),
    )));

    assert_ne!(converted.preedit, "qq");
    assert_eq!(
        converted.candidate_window,
        protocol::CandidateWindow::Hidden as i32
    );
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
            action: protocol::UserAction::Input as i32,
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
            action: protocol::UserAction::Enter as i32,
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
            action: protocol::UserAction::Input as i32,
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
            action: protocol::UserAction::Input as i32,
            shift: false,
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
            action: protocol::UserAction::Input as i32,
            shift: false,
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
