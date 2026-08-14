mod dictionary;
mod input;
mod lattice;
mod learning;
mod post_prediction;
mod session;
mod special;
mod template;
mod text_replacer;
mod typo;

pub use dictionary::{
    CharacterIdMap, DictionaryBinaryError, DictionaryEntry, DictionaryError, DictionaryMatch,
    DictionaryMetadata, DictionaryStore, Louds, LoudsError, MeaningMatrix, UserDictionary,
    escaped_identifier, parse_connection_cost_line, parse_entry_block, parse_entry_shard,
};
pub use input::{
    ComposingCount, ComposingText, DifferenceSuffix, FormatError, FormatErrorKind, FormatReport,
    FormatSide, InputElement, InputModifier, InputPiece, InputStyle, InputTable,
    InputTableExportError, InputTableId, InputTableRegistry, KeyElement, ValueElement,
};
pub use lattice::{Candidate, CompleteAction, ConversionContext, LatticeRange, NormalConverter};
pub use learning::{LearningError, LearningMemory, LearningMode};
pub use post_prediction::{
    PostCompositionPrediction, PostCompositionPredictor, PostPredictionKind,
};
pub use session::{
    ConversionResult, ConversionSession, PredictionMode, RequestOptions, SelectionError,
    TypoCorrectionMode,
};
pub use special::special_candidates;
pub use template::expand_templates;
pub use text_replacer::{ReplacementCandidate, TextReplacer, TextReplacerError, TextSearchResult};
