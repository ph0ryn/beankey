use std::error::Error;
use std::fmt;

use super::{DictionaryEntry, DictionaryMetadata};

const ENTRY_HEADER_SIZE: usize = 10;
const MEANING_COUNT: usize = 502;
const CONNECTION_COUNT: usize = 1319;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DictionaryBinaryError {
    Truncated {
        context: &'static str,
        needed: usize,
        available: usize,
    },
    InvalidShardIndex {
        index: usize,
        slot_count: usize,
    },
    InvalidOffset {
        start: usize,
        end: usize,
        length: usize,
    },
    MissingEntryFields {
        expected: usize,
        found: usize,
    },
    InvalidConnectionId(i32),
    InvalidMeaningMatrixLength {
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for DictionaryBinaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                context,
                needed,
                available,
            } => write!(
                formatter,
                "truncated {context}: needed {needed} bytes, found {available}"
            ),
            Self::InvalidShardIndex { index, slot_count } => {
                write!(
                    formatter,
                    "shard index {index} is outside {slot_count} slots"
                )
            }
            Self::InvalidOffset { start, end, length } => write!(
                formatter,
                "invalid dictionary block offset {start}..{end} for {length} bytes"
            ),
            Self::MissingEntryFields { expected, found } => write!(
                formatter,
                "dictionary block requires {expected} text fields but contains {found}"
            ),
            Self::InvalidConnectionId(value) => {
                write!(formatter, "invalid connection identifier {value}")
            }
            Self::InvalidMeaningMatrixLength { expected, found } => write!(
                formatter,
                "meaning matrix requires at least {expected} floats but contains {found}"
            ),
        }
    }
}

impl Error for DictionaryBinaryError {}

pub fn parse_entry_block(binary: &[u8]) -> Result<Vec<DictionaryEntry>, DictionaryBinaryError> {
    let count = read_u16(binary, 0, "entry count")? as usize;
    let text_offset = 2 + count * ENTRY_HEADER_SIZE;
    require(binary, text_offset, "entry headers")?;

    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 2 + index * ENTRY_HEADER_SIZE;
        entries.push(DictionaryEntry {
            word: String::new(),
            ruby: String::new(),
            left_id: read_u16(binary, offset, "left connection identifier")?,
            right_id: read_u16(binary, offset + 2, "right connection identifier")?,
            meaning_id: read_u16(binary, offset + 4, "meaning identifier")?,
            base_value: read_f32(binary, offset + 6, "entry value")?,
            adjustment: 0.0,
            metadata: DictionaryMetadata::default(),
        });
    }

    let fields: Vec<_> = binary[text_offset..].split(|byte| *byte == b'\t').collect();
    let expected = count + 1;
    if fields.len() < expected {
        return Err(DictionaryBinaryError::MissingEntryFields {
            expected,
            found: fields.len(),
        });
    }
    let ruby = String::from_utf8_lossy(fields[0]).into_owned();
    for (entry, word) in entries.iter_mut().zip(&fields[1..=count]) {
        entry.ruby.clone_from(&ruby);
        entry.word = if word.is_empty() {
            ruby.clone()
        } else {
            String::from_utf8_lossy(word).into_owned()
        };
    }
    Ok(entries)
}

pub fn parse_entry_shard(
    binary: &[u8],
    indices: impl IntoIterator<Item = usize>,
) -> Result<Vec<DictionaryEntry>, DictionaryBinaryError> {
    let slot_count = read_u16(binary, 0, "shard slot count")? as usize;
    require(binary, 2 + slot_count * 4, "shard offset table")?;
    let mut output = Vec::new();
    for index in indices {
        if index >= slot_count {
            return Err(DictionaryBinaryError::InvalidShardIndex { index, slot_count });
        }
        let start = read_u32(binary, 2 + index * 4, "shard block offset")? as usize;
        let end = if index + 1 == slot_count {
            binary.len()
        } else {
            read_u32(binary, 2 + (index + 1) * 4, "shard block offset")? as usize
        };
        if start > end || end > binary.len() {
            return Err(DictionaryBinaryError::InvalidOffset {
                start,
                end,
                length: binary.len(),
            });
        }
        output.extend(parse_entry_block(&binary[start..end])?);
    }
    Ok(output)
}

pub fn parse_connection_cost_line(binary: &[u8]) -> Result<Vec<f32>, DictionaryBinaryError> {
    require(binary, 8, "connection cost default record")?;
    let mut records = binary.chunks_exact(8);
    if !records.remainder().is_empty() {
        return Err(DictionaryBinaryError::Truncated {
            context: "connection cost record",
            needed: binary.len().next_multiple_of(8),
            available: binary.len(),
        });
    }
    let first = records.next().expect("length checked");
    let default_id = i32::from_le_bytes(first[..4].try_into().expect("record width"));
    if default_id != -1 {
        return Err(DictionaryBinaryError::InvalidConnectionId(default_id));
    }
    let default_value = f32::from_le_bytes(first[4..].try_into().expect("record width"));
    let mut line = vec![default_value; CONNECTION_COUNT];
    for record in records {
        let id = i32::from_le_bytes(record[..4].try_into().expect("record width"));
        let Ok(index) = usize::try_from(id) else {
            return Err(DictionaryBinaryError::InvalidConnectionId(id));
        };
        let Some(value) = line.get_mut(index) else {
            return Err(DictionaryBinaryError::InvalidConnectionId(id));
        };
        *value = f32::from_le_bytes(record[4..].try_into().expect("record width"));
    }
    Ok(line)
}

#[derive(Clone, Debug)]
pub struct MeaningMatrix {
    values: Vec<f32>,
}

impl MeaningMatrix {
    pub fn parse(binary: &[u8]) -> Result<Self, DictionaryBinaryError> {
        let values: Vec<_> = binary
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunk width")))
            .collect();
        let expected = MEANING_COUNT * MEANING_COUNT;
        if !binary.chunks_exact(4).remainder().is_empty() || values.len() < expected {
            return Err(DictionaryBinaryError::InvalidMeaningMatrixLength {
                expected,
                found: values.len(),
            });
        }
        Ok(Self { values })
    }

    pub fn get(&self, former: usize, latter: usize) -> Option<f32> {
        if former == 500 || latter == 500 {
            return Some(0.0);
        }
        self.values.get(former * MEANING_COUNT + latter).copied()
    }
}

fn require(
    binary: &[u8],
    needed: usize,
    context: &'static str,
) -> Result<(), DictionaryBinaryError> {
    if binary.len() < needed {
        Err(DictionaryBinaryError::Truncated {
            context,
            needed,
            available: binary.len(),
        })
    } else {
        Ok(())
    }
}

fn read_u16(
    binary: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<u16, DictionaryBinaryError> {
    require(binary, offset + 2, context)?;
    Ok(u16::from_le_bytes(
        binary[offset..offset + 2]
            .try_into()
            .expect("slice width checked"),
    ))
}

fn read_u32(
    binary: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<u32, DictionaryBinaryError> {
    require(binary, offset + 4, context)?;
    Ok(u32::from_le_bytes(
        binary[offset..offset + 4]
            .try_into()
            .expect("slice width checked"),
    ))
}

fn read_f32(
    binary: &[u8],
    offset: usize,
    context: &'static str,
) -> Result<f32, DictionaryBinaryError> {
    Ok(f32::from_bits(read_u32(binary, offset, context)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_block() -> Vec<u8> {
        let mut binary = Vec::new();
        binary.extend(2_u16.to_le_bytes());
        for (left, right, meaning, value) in [
            (10_u16, 11_u16, 12_u16, -3.5_f32),
            (20_u16, 21_u16, 22_u16, -7.25_f32),
        ] {
            binary.extend(left.to_le_bytes());
            binary.extend(right.to_le_bytes());
            binary.extend(meaning.to_le_bytes());
            binary.extend(value.to_le_bytes());
        }
        binary.extend("カナ\t仮名\t".as_bytes());
        binary
    }

    #[test]
    fn parses_entry_headers_and_shared_ruby_payload() {
        let entries = parse_entry_block(&entry_block()).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].word, "仮名");
        assert_eq!(entries[0].ruby, "カナ");
        assert_eq!((entries[0].left_id, entries[0].right_id), (10, 11));
        assert_eq!(entries[0].meaning_id, 12);
        assert_eq!(entries[0].base_value, -3.5);
        assert_eq!(entries[1].word, "カナ");
        assert_eq!(entries[1].base_value, -7.25);
    }

    #[test]
    fn parses_selected_shard_slots() {
        let block = entry_block();
        let empty = [0_u8, 0, b'X', b'\t'];
        let header_len = 2 + 2 * 4;
        let mut shard = Vec::new();
        shard.extend(2_u16.to_le_bytes());
        shard.extend((header_len as u32).to_le_bytes());
        shard.extend(((header_len + empty.len()) as u32).to_le_bytes());
        shard.extend(empty);
        shard.extend(block);

        let entries = parse_entry_shard(&shard, [1]).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].word, "仮名");
    }

    #[test]
    fn expands_sparse_connection_costs() {
        let mut binary = Vec::new();
        binary.extend((-1_i32).to_le_bytes());
        binary.extend((-25_f32).to_le_bytes());
        binary.extend(42_i32.to_le_bytes());
        binary.extend((-1.5_f32).to_le_bytes());

        let line = parse_connection_cost_line(&binary).unwrap();

        assert_eq!(line.len(), CONNECTION_COUNT);
        assert_eq!(line[0], -25.0);
        assert_eq!(line[42], -1.5);
    }

    #[test]
    fn bos_and_eos_meaning_cost_is_always_zero() {
        let matrix = MeaningMatrix {
            values: vec![-1.0; MEANING_COUNT * MEANING_COUNT],
        };

        assert_eq!(matrix.get(500, 1), Some(0.0));
        assert_eq!(matrix.get(1, 500), Some(0.0));
        assert_eq!(matrix.get(1, 2), Some(-1.0));
    }
}
