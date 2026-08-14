use std::path::PathBuf;

use beankey_converter::{
    ConversionSession, DictionaryStore, InputStyle, InputTableRegistry, NormalConverter,
    PostCompositionPredictor,
};

fn dictionary_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/azooKey_dictionary_storage/Dictionary")
}

#[test]
fn reads_fixed_zero_hint_assets_and_predicts_after_commit() {
    let dictionary = DictionaryStore::open(dictionary_root()).unwrap();
    let converter = NormalConverter::new(&dictionary);
    let predictor = PostCompositionPredictor::new(&dictionary);
    let tables = InputTableRegistry::new();
    let mut session = ConversionSession::new();
    session.insert_str("きょう", InputStyle::Direct, &tables);

    let candidates = session.request_candidates(&converter, &tables, 10).unwrap();
    let index = candidates
        .iter()
        .position(|candidate| candidate.text == "今日")
        .unwrap();
    let selected = candidates[index].clone();
    session.select_candidate(index, &tables).unwrap();
    let predictions = session
        .request_post_composition_predictions(&predictor)
        .unwrap();

    assert!(!predictions.is_empty());
    assert!(predictions.len() <= 10);
    assert!(predictions.iter().any(|prediction| {
        let joined = prediction.join(&selected);
        joined.text.starts_with("今日") && joined.text.len() > "今日".len()
    }));
}
