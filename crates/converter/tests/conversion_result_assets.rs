use std::path::PathBuf;

use beankey_converter::{
    ConversionSession, DictionaryStore, InputStyle, InputTableRegistry, NormalConverter,
    PredictionMode, RequestOptions,
};

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

#[test]
fn assembles_full_clause_word_representation_and_prediction_groups() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut session = ConversionSession::new();
    session.insert_str("kyo", InputStyle::RomanToKana, &tables);

    let result = session
        .request(
            &converter,
            &tables,
            RequestOptions {
                japanese_prediction: PredictionMode::Manual,
                half_width_kana: true,
                ..RequestOptions::default()
            },
        )
        .unwrap();

    assert!(
        result
            .main_results
            .iter()
            .any(|candidate| candidate.text == "巨")
    );
    assert!(
        result
            .main_results
            .iter()
            .any(|candidate| candidate.text == "キョ")
    );
    assert!(
        result
            .main_results
            .iter()
            .any(|candidate| candidate.text == "きょ")
    );
    assert!(
        result
            .main_results
            .iter()
            .any(|candidate| candidate.text == "ｷｮ")
    );
    assert!(
        result
            .prediction_results
            .iter()
            .any(|candidate| candidate.text == "今日")
    );
    assert!(result.english_prediction_results.is_empty());
    assert_eq!(session.candidates(), result.main_results);
}
