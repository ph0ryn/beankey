mod binary;
mod entry;

pub use binary::{
    DictionaryBinaryError, MeaningMatrix, parse_connection_cost_line, parse_entry_block,
    parse_entry_shard,
};
pub use entry::{DictionaryEntry, DictionaryMetadata};
