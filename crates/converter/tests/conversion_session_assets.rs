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
fn selects_and_commits_a_candidate_without_leaking_between_sessions() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut first = ConversionSession::new();
    let mut second = ConversionSession::new();
    first.insert_str("shikai", InputStyle::RomanToKana, &tables);
    second.insert_str("かな", InputStyle::Direct, &tables);

    let candidates = first.request_candidates(&converter, &tables, 10).unwrap();
    let index = candidates
        .iter()
        .position(|candidate| candidate.text == "司会")
        .unwrap();
    let committed = first.select_candidate(index, &tables).unwrap();

    assert_eq!(committed, "司会");
    assert!(first.composing().is_empty());
    assert_eq!(second.composing().surface(), "かな");
    assert!(second.candidates().is_empty());
}
