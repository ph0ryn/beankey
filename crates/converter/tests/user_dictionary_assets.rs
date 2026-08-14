use std::path::PathBuf;

use beankey_converter::{
    ConversionSession, DictionaryEntry, DictionaryMetadata, DictionaryStore, InputStyle,
    InputTableRegistry, NormalConverter, PredictionMode, RequestOptions,
};

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

fn user_entry(word: &str, ruby: &str) -> DictionaryEntry {
    DictionaryEntry {
        word: word.into(),
        ruby: ruby.into(),
        left_id: 1288,
        right_id: 1288,
        meaning_id: 501,
        base_value: -1.0,
        adjustment: 0.0,
        metadata: DictionaryMetadata::default(),
    }
}

#[test]
fn uses_dynamic_entries_for_conversion_prediction_and_shortcuts() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut session = ConversionSession::new();
    session.import_dynamic_user_dictionary(
        vec![user_entry("Codex", "コーデックス")],
        vec![user_entry("openai.com", "オープンエーアイ")],
    );

    session.insert_str("こーでっくす", InputStyle::Direct, &tables);
    let result = session
        .request(
            &converter,
            &tables,
            RequestOptions {
                japanese_prediction: PredictionMode::Disabled,
                ..RequestOptions::default()
            },
        )
        .unwrap();
    let custom = result
        .main_results
        .iter()
        .find(|candidate| candidate.text == "Codex")
        .unwrap();
    assert!(
        custom.entries[0]
            .metadata
            .contains(DictionaryMetadata::USER_DICTIONARY)
    );

    session.reset();
    session.insert_str("こー", InputStyle::Direct, &tables);
    assert!(
        session
            .request_predictions(&converter, &tables, 10)
            .unwrap()
            .iter()
            .any(|candidate| candidate.text == "Codex")
    );

    session.reset();
    session.insert_str("おーぷんえーあい", InputStyle::Direct, &tables);
    assert!(
        session
            .request(&converter, &tables, RequestOptions::default())
            .unwrap()
            .main_results
            .iter()
            .any(|candidate| candidate.text == "openai.com")
    );
}

#[test]
fn expands_dynamic_dictionary_templates_before_exposing_candidates() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut session = ConversionSession::new();
    session.import_dynamic_user_dictionary(
        vec![user_entry(
            "<random type=\"int\" value=\"7,7\">",
            "テンプレート",
        )],
        Vec::new(),
    );

    session.insert_str("てんぷれーと", InputStyle::Direct, &tables);
    let result = session
        .request(&converter, &tables, RequestOptions::default())
        .unwrap();
    let candidate = result
        .main_results
        .iter()
        .find(|candidate| candidate.text == "7")
        .expect("expanded template candidate");
    assert!(!candidate.is_learning_target);
    assert!(
        !result
            .main_results
            .iter()
            .any(|candidate| candidate.text.starts_with("<random"))
    );
}
