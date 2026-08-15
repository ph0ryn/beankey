use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoudsError {
    InvalidWordLength(usize),
    MissingZeroDelimiters { needed: usize, found: usize },
    NodeTableTooLarge(usize),
    InvalidTopology { node: usize, zero_position: usize },
}

impl fmt::Display for LoudsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWordLength(length) => {
                write!(
                    formatter,
                    "LOUDS bit data length {length} is not divisible by 8"
                )
            }
            Self::MissingZeroDelimiters { needed, found } => write!(
                formatter,
                "LOUDS data requires {needed} zero delimiters but contains {found}"
            ),
            Self::NodeTableTooLarge(length) => {
                write!(
                    formatter,
                    "character ID table contains {length} entries; maximum is 256"
                )
            }
            Self::InvalidTopology {
                node,
                zero_position,
            } => write!(
                formatter,
                "LOUDS node {node} has invalid zero position {zero_position}"
            ),
        }
    }
}

impl Error for LoudsError {}

#[derive(Clone, Debug)]
pub struct CharacterIdMap {
    ids: HashMap<String, u8>,
}

impl CharacterIdMap {
    pub fn parse(content: &str) -> Result<Self, LoudsError> {
        let graphemes: Vec<_> = UnicodeSegmentation::graphemes(content, true).collect();
        if graphemes.len() > 256 {
            return Err(LoudsError::NodeTableTooLarge(graphemes.len()));
        }
        Ok(Self {
            ids: graphemes
                .into_iter()
                .enumerate()
                .map(|(index, value)| (value.to_owned(), index as u8))
                .collect(),
        })
    }

    pub fn id(&self, grapheme: &str) -> Option<u8> {
        self.ids.get(grapheme).copied()
    }

    pub fn encode(&self, value: &str) -> Option<Vec<u8>> {
        UnicodeSegmentation::graphemes(value, true)
            .map(|value| self.id(value))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Louds {
    node_ids: Vec<u8>,
    child_ranges: Vec<Range<usize>>,
}

impl Louds {
    pub fn parse(bits: &[u8], node_ids: &[u8]) -> Result<Self, LoudsError> {
        if bits.len() % 8 != 0 {
            return Err(LoudsError::InvalidWordLength(bits.len()));
        }

        let needed_zeros = node_ids.len();
        let mut zero_positions = Vec::with_capacity(needed_zeros);
        let mut bit_position = 0_usize;
        for chunk in bits.chunks_exact(8) {
            let word = u64::from_le_bytes(chunk.try_into().expect("chunk width"));
            for shift in (0..64).rev() {
                if word & (1_u64 << shift) == 0 {
                    zero_positions.push(bit_position);
                    if zero_positions.len() == needed_zeros {
                        break;
                    }
                }
                bit_position += 1;
            }
            if zero_positions.len() == needed_zeros {
                break;
            }
        }
        if zero_positions.len() < needed_zeros {
            return Err(LoudsError::MissingZeroDelimiters {
                needed: needed_zeros,
                found: zero_positions.len(),
            });
        }

        let mut child_ranges = vec![0..0; node_ids.len()];
        for node in 1..node_ids.len() {
            let start = zero_positions[node - 1]
                .checked_sub(node)
                .and_then(|value| value.checked_add(2))
                .ok_or(LoudsError::InvalidTopology {
                    node,
                    zero_position: zero_positions[node - 1],
                })?;
            let end = zero_positions[node]
                .checked_sub(node)
                .and_then(|value| value.checked_add(1))
                .ok_or(LoudsError::InvalidTopology {
                    node,
                    zero_position: zero_positions[node],
                })?;
            child_ranges[node] = start.min(node_ids.len())..end.min(node_ids.len());
        }
        Ok(Self {
            node_ids: node_ids.to_vec(),
            child_ranges,
        })
    }

    pub fn child_range(&self, parent: usize) -> Range<usize> {
        self.child_ranges.get(parent).cloned().unwrap_or(0..0)
    }

    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }

    pub fn search_child(&self, parent: usize, character_id: u8) -> Option<usize> {
        self.child_range(parent)
            .find(|index| self.node_ids.get(*index) == Some(&character_id))
    }

    pub fn search(&self, character_ids: &[u8]) -> Option<usize> {
        let mut node = 1;
        for character_id in character_ids {
            node = self.search_child(node, *character_id)?;
        }
        Some(node)
    }

    pub fn descendants(
        &self,
        character_ids: &[u8],
        max_depth: usize,
        max_count: usize,
    ) -> Vec<usize> {
        let Some(root) = self.search(character_ids) else {
            return Vec::new();
        };
        self.descendants_from(root, 0, max_depth, max_count)
    }

    fn descendants_from(
        &self,
        parent: usize,
        depth: usize,
        max_depth: usize,
        max_count: usize,
    ) -> Vec<usize> {
        let direct: Vec<_> = self.child_range(parent).collect();
        let mut output = direct.clone();
        if depth == max_depth {
            return output;
        }
        for child in direct {
            if output.len() > max_count {
                break;
            }
            output.extend(self.descendants_from(
                child,
                depth + 1,
                max_depth,
                max_count.saturating_sub(output.len()),
            ));
        }
        output
    }
}

pub fn escaped_identifier(value: &str) -> String {
    if matches!(value, "user" | "memory" | "user_shortcuts") {
        return value.to_owned();
    }
    let chunks: Vec<_> = value
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect();
    format!("[{}]", chunks.join("_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_louds(bits: &[bool]) -> Vec<u8> {
        let mut words = Vec::new();
        for chunk in bits.chunks(64) {
            let mut word = 0_u64;
            for (index, value) in chunk.iter().enumerate() {
                if *value {
                    word |= 1 << (63 - index);
                }
            }
            if chunk.len() < 64 {
                for index in chunk.len()..64 {
                    word |= 1 << (63 - index);
                }
            }
            words.extend(word.to_le_bytes());
        }
        words
    }

    #[test]
    fn traverses_a_level_order_unary_tree() {
        // Super-root, root, then one child for each of the first two nodes.
        let bits = encoded_louds(&[true, false, true, false, true, false, false]);
        let louds = Louds::parse(&bits, &[0, 0, 10, 20]).unwrap();

        assert_eq!(louds.child_range(1), 2..3);
        assert_eq!(louds.child_range(2), 3..4);
        assert_eq!(louds.search(&[10, 20]), Some(3));
        assert_eq!(louds.search(&[20]), None);
    }

    #[test]
    fn rejects_an_invalid_topology_without_panicking() {
        assert!(matches!(
            Louds::parse(&[0; 8], &[0, 0, 0]),
            Err(LoudsError::InvalidTopology { .. })
        ));
    }

    #[test]
    fn encodes_filesystem_identifiers_as_utf16_units() {
        assert_eq!(escaped_identifier("あ"), "[3042]");
        assert_eq!(escaped_identifier("AB"), "[0041_0042]");
        assert_eq!(escaped_identifier("🇯🇵"), "[D83C_DDEF_D83C_DDF5]");
        assert_eq!(escaped_identifier("memory"), "memory");
    }

    #[test]
    fn enumerates_character_ids_by_extended_grapheme() {
        let map = CharacterIdMap::parse("アか\u{3099}😀").unwrap();

        assert_eq!(map.len(), 3);
        assert_eq!(map.encode("か\u{3099}😀"), Some(vec![1, 2]));
    }
}
