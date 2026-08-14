use std::path::PathBuf;

use beankey_converter::{
    ConversionSession, DictionaryStore, InputStyle, InputTableRegistry, NormalConverter,
};

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

#[test]
fn predicts_longer_words_from_a_direct_kana_prefix() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut session = ConversionSession::new();
    session.insert_str("きょ", InputStyle::Direct, &tables);

    let predictions = session
        .request_predictions(&converter, &tables, 10)
        .unwrap();

    assert!(predictions.iter().any(|candidate| candidate.text == "今日"));
    assert!(predictions.iter().all(|candidate| candidate.ruby_count > 2));
    assert!(
        predictions
            .windows(2)
            .all(|pair| pair[0].value >= pair[1].value)
    );
}

#[test]
fn does_not_predict_from_only_an_unresolved_roman_prefix() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut session = ConversionSession::new();
    session.insert_str("ky", InputStyle::RomanToKana, &tables);

    assert!(
        session
            .request_predictions(&converter, &tables, 10)
            .unwrap()
            .is_empty()
    );
}
