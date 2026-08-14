use std::path::PathBuf;

use beankey_converter::{
    ComposingCount, ComposingText, DictionaryStore, InputStyle, InputTableRegistry, NormalConverter,
};

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

#[test]
fn converts_direct_kana_with_the_fixed_dictionary() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut composing = ComposingText::new();
    composing.insert_str("しかい", InputStyle::Direct, &tables);

    let candidates = converter.convert(&composing, &tables, 10).unwrap();

    assert!(candidates.iter().any(|candidate| candidate.text == "司会"));
    assert!(candidates.iter().any(|candidate| candidate.text == "視界"));
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.composing_count == ComposingCount::Surface(3))
    );
    assert!(
        candidates
            .windows(2)
            .all(|pair| pair[0].value >= pair[1].value)
    );
}

#[test]
fn converts_roman_history_through_the_surface_lattice() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut composing = ComposingText::new();
    composing.insert_str("shikai", InputStyle::RomanToKana, &tables);
    assert_eq!(composing.surface(), "しかい");

    let candidates = converter.convert(&composing, &tables, 10).unwrap();

    assert!(candidates.iter().any(|candidate| candidate.text == "司会"));
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.composing_count == ComposingCount::Surface(3))
    );
}
