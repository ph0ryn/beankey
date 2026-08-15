use std::cmp::Ordering;
use std::collections::BTreeSet;

use unicode_segmentation::UnicodeSegmentation;

use super::roman::DEFAULT_ROMAN_TO_KANA;
use crate::kana::to_katakana;

pub type Grapheme = String;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputModifier {
    Shift,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum InputPiece {
    Character(Grapheme),
    CompositionSeparator,
    Key {
        intention: Option<Grapheme>,
        input: Grapheme,
        modifiers: BTreeSet<InputModifier>,
    },
}

impl InputPiece {
    pub fn character(value: impl Into<String>) -> Self {
        Self::Character(value.into())
    }

    pub(crate) fn displayed(&self) -> Option<&str> {
        match self {
            Self::Character(value) => Some(value),
            Self::CompositionSeparator => None,
            Self::Key {
                intention, input, ..
            } => Some(intention.as_deref().unwrap_or(input)),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum KeyElement {
    Piece(InputPiece),
    Any,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ValueElement {
    Character(Grapheme),
    Any,
}

#[derive(Clone, Debug)]
struct Mapping {
    key: Vec<KeyElement>,
    value: Vec<ValueElement>,
}

#[derive(Clone, Debug, Default)]
pub struct InputTable {
    mappings: Vec<Mapping>,
    max_key_count: usize,
}

#[derive(Clone, Debug)]
struct Match {
    mapping_index: usize,
    depth: usize,
    wildcard_count: usize,
    exact_key_count: usize,
    resolved_wildcard: Option<InputPiece>,
}

impl InputTable {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn new(entries: impl IntoIterator<Item = (Vec<KeyElement>, Vec<ValueElement>)>) -> Self {
        let mut mappings: Vec<Mapping> = Vec::new();
        for (key, value) in entries {
            if let Some(existing) = mappings.iter_mut().find(|mapping| mapping.key == key) {
                existing.value = value;
            } else {
                mappings.push(Mapping { key, value });
            }
        }
        let max_key_count = mappings
            .iter()
            .map(|mapping| mapping.key.len())
            .max()
            .unwrap_or(0);
        Self {
            mappings,
            max_key_count,
        }
    }

    pub fn default_roman_to_kana() -> Self {
        let mut entries: Vec<_> = DEFAULT_ROMAN_TO_KANA
            .iter()
            .map(|(key, value)| {
                (
                    graphemes(key)
                        .into_iter()
                        .map(|value| KeyElement::Piece(InputPiece::Character(value)))
                        .collect(),
                    graphemes(value)
                        .into_iter()
                        .map(ValueElement::Character)
                        .collect(),
                )
            })
            .collect();
        entries.extend([
            (
                vec![KeyElement::Piece(InputPiece::CompositionSeparator)],
                vec![],
            ),
            (
                vec![
                    KeyElement::Piece(InputPiece::character("n")),
                    KeyElement::Piece(InputPiece::CompositionSeparator),
                ],
                vec![ValueElement::Character("ん".into())],
            ),
            (
                vec![
                    KeyElement::Piece(InputPiece::character("n")),
                    KeyElement::Any,
                ],
                vec![ValueElement::Character("ん".into()), ValueElement::Any],
            ),
        ]);
        Self::new(entries)
    }

    pub fn default_azik() -> Self {
        let mut entries = simple_tsv_entries(include_str!("azik.tsv"));
        entries.push((
            vec![KeyElement::Piece(InputPiece::CompositionSeparator)],
            vec![],
        ));
        Self::new(entries)
    }

    pub fn default_kana_jis() -> Self {
        let mut entries = simple_tsv_entries(include_str!("kana_jis.tsv"));
        entries.extend([
            (
                vec![KeyElement::Piece(InputPiece::CompositionSeparator)],
                vec![],
            ),
            (
                vec![KeyElement::Piece(InputPiece::Key {
                    intention: Some("0".into()),
                    input: "0".into(),
                    modifiers: BTreeSet::from([InputModifier::Shift]),
                })],
                vec![ValueElement::Character("を".into())],
            ),
        ]);
        Self::new(entries)
    }

    pub fn default_kana_us() -> Self {
        let mut entries = simple_tsv_entries(include_str!("kana_us.tsv"));
        entries.push((
            vec![KeyElement::Piece(InputPiece::CompositionSeparator)],
            vec![],
        ));
        Self::new(entries)
    }

    pub fn combined(tables: impl IntoIterator<Item = Self>) -> Self {
        Self::new(tables.into_iter().flat_map(|table| {
            table
                .mappings
                .into_iter()
                .map(|mapping| (mapping.key, mapping.value))
        }))
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&[KeyElement], &[ValueElement])> {
        self.mappings
            .iter()
            .map(|mapping| (mapping.key.as_slice(), mapping.value.as_slice()))
    }

    pub fn apply(&self, buffer: &mut Vec<Grapheme>, added: InputPiece) -> usize {
        let best = self
            .mappings
            .iter()
            .enumerate()
            .filter_map(|(mapping_index, mapping)| {
                self.match_mapping(mapping_index, mapping, buffer, &added)
            })
            .max_by(compare_matches);

        if let Some(best) = best {
            let mapping = &self.mappings[best.mapping_index];
            let delete_count = best.depth.saturating_sub(1);
            buffer.truncate(buffer.len().saturating_sub(delete_count));
            for element in &mapping.value {
                match element {
                    ValueElement::Character(value) => buffer.push(value.clone()),
                    ValueElement::Any => {
                        if let Some(value) = best
                            .resolved_wildcard
                            .as_ref()
                            .and_then(InputPiece::displayed)
                        {
                            buffer.push(value.to_owned());
                        }
                    }
                }
            }
            return delete_count;
        }

        if let Some(value) = added.displayed() {
            buffer.push(value.to_owned());
        }
        0
    }

    pub fn applied(&self, current_text: &str, added: InputPiece) -> String {
        let mut buffer = graphemes(current_text);
        self.apply(&mut buffer, added);
        buffer.concat()
    }

    pub fn possible_nexts(&self, prefix: &str) -> Vec<String> {
        self.mappings
            .iter()
            .filter_map(|mapping| {
                let key: Option<String> = mapping
                    .key
                    .iter()
                    .map(|element| match element {
                        KeyElement::Piece(InputPiece::Character(value)) => Some(value.as_str()),
                        _ => None,
                    })
                    .collect();
                let key = key?;
                if prefix.is_empty()
                    || prefix.chars().count() >= key.chars().count()
                    || !key.starts_with(prefix)
                {
                    return None;
                }
                let value: Option<String> = mapping
                    .value
                    .iter()
                    .map(|element| match element {
                        ValueElement::Character(value) => Some(value.as_str()),
                        ValueElement::Any => None,
                    })
                    .collect();
                value.map(|value| to_katakana(&value))
            })
            .collect()
    }

    fn match_mapping(
        &self,
        mapping_index: usize,
        mapping: &Mapping,
        buffer: &[Grapheme],
        added: &InputPiece,
    ) -> Option<Match> {
        if mapping.key.is_empty() || mapping.key.len() > self.max_key_count {
            return None;
        }
        let mut resolved_wildcard: Option<InputPiece> = None;
        let mut wildcard_count = 0;
        let mut exact_key_count = 0;

        for (depth, expected) in mapping.key.iter().rev().enumerate() {
            let current = if depth == 0 {
                added.clone()
            } else {
                let index = buffer.len().checked_sub(depth)?;
                InputPiece::Character(buffer[index].clone())
            };
            match expected {
                KeyElement::Any => {
                    if let Some(previous) = &resolved_wildcard {
                        if previous != &current {
                            return None;
                        }
                    } else {
                        resolved_wildcard = Some(current);
                    }
                    wildcard_count += 1;
                }
                KeyElement::Piece(expected) => {
                    if !piece_matches(expected, &current, &mut exact_key_count) {
                        return None;
                    }
                }
            }
        }

        Some(Match {
            mapping_index,
            depth: mapping.key.len(),
            wildcard_count,
            exact_key_count,
            resolved_wildcard,
        })
    }
}

fn piece_matches(expected: &InputPiece, current: &InputPiece, exact_key_count: &mut usize) -> bool {
    match (expected, current) {
        (InputPiece::Character(expected), current) => current.displayed() == Some(expected),
        (InputPiece::CompositionSeparator, InputPiece::CompositionSeparator) => true,
        (
            InputPiece::Key {
                input: expected_input,
                modifiers: expected_modifiers,
                ..
            },
            InputPiece::Key {
                intention,
                input,
                modifiers,
            },
        ) if intention.as_deref().unwrap_or(input) == expected_input
            && modifiers == expected_modifiers =>
        {
            *exact_key_count += 1;
            true
        }
        _ => false,
    }
}

fn compare_matches(left: &Match, right: &Match) -> Ordering {
    left.depth
        .cmp(&right.depth)
        .then_with(|| right.wildcard_count.cmp(&left.wildcard_count))
        .then_with(|| left.exact_key_count.cmp(&right.exact_key_count))
        .then_with(|| right.mapping_index.cmp(&left.mapping_index))
}

pub(crate) fn graphemes(value: &str) -> Vec<Grapheme> {
    UnicodeSegmentation::graphemes(value, true)
        .map(ToOwned::to_owned)
        .collect()
}

fn simple_tsv_entries(source: &str) -> Vec<(Vec<KeyElement>, Vec<ValueElement>)> {
    source
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(key, value)| {
            (
                graphemes(key)
                    .into_iter()
                    .map(|value| KeyElement::Piece(InputPiece::Character(value)))
                    .collect(),
                graphemes(value)
                    .into_iter()
                    .map(ValueElement::Character)
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(table: &InputTable, input: &str) -> String {
        let mut buffer = Vec::new();
        for value in graphemes(input) {
            table.apply(&mut buffer, InputPiece::Character(value));
        }
        buffer.concat()
    }

    #[test]
    fn converts_standard_roman_sequences_incrementally() {
        let table = InputTable::default_roman_to_kana();

        assert_eq!(type_text(&table, "kyouhaame"), "きょうはあめ");
        assert_eq!(type_text(&table, "kansha"), "かんしゃ");
        assert_eq!(type_text(&table, "itta"), "いった");
    }

    #[test]
    fn composition_separator_resolves_trailing_n() {
        let table = InputTable::default_roman_to_kana();
        let mut buffer = Vec::new();
        for value in graphemes("kan") {
            table.apply(&mut buffer, InputPiece::Character(value));
        }

        table.apply(&mut buffer, InputPiece::CompositionSeparator);

        assert_eq!(buffer.concat(), "かん");
    }

    #[test]
    fn concrete_mapping_wins_over_wildcard() {
        let table = InputTable::default_roman_to_kana();

        assert_eq!(type_text(&table, "nya"), "にゃ");
        assert_eq!(type_text(&table, "nba"), "んば");
    }

    #[test]
    fn wildcard_preserves_an_extended_grapheme_cluster() {
        let table = InputTable::new([(vec![KeyElement::Any], vec![ValueElement::Any])]);

        assert_eq!(
            table.applied("", InputPiece::character("か\u{3099}")),
            "か\u{3099}"
        );
    }

    #[test]
    fn key_rule_beats_character_rule_at_equal_depth() {
        let shifted = InputPiece::Key {
            intention: Some("_".into()),
            input: "_".into(),
            modifiers: BTreeSet::from([InputModifier::Shift]),
        };
        let table = InputTable::new([
            (
                vec![KeyElement::Piece(InputPiece::character("_"))],
                vec![ValueElement::Character("字".into())],
            ),
            (
                vec![KeyElement::Piece(shifted.clone())],
                vec![ValueElement::Character("鍵".into())],
            ),
        ]);

        assert_eq!(table.applied("", shifted), "鍵");
    }

    #[test]
    fn later_tables_replace_duplicate_rules() {
        let first = InputTable::new([(
            vec![KeyElement::Piece(InputPiece::character("a"))],
            vec![ValueElement::Character("あ".into())],
        )]);
        let second = InputTable::new([(
            vec![KeyElement::Piece(InputPiece::character("a"))],
            vec![ValueElement::Character("安".into())],
        )]);

        assert_eq!(
            InputTable::combined([first, second]).applied("", InputPiece::character("a")),
            "安"
        );
    }

    #[test]
    fn converts_azik_sequences() {
        let table = InputTable::default_azik();

        assert_eq!(type_text(&table, "szzz"), "さんざん");
        assert_eq!(type_text(&table, "kz"), "かん");
        assert_eq!(type_text(&table, "ds"), "です");
    }

    #[test]
    fn converts_jis_kana_keys_and_shift_zero() {
        let table = InputTable::default_kana_jis();
        assert_eq!(type_text(&table, "qwerty"), "たていすかん");

        let shifted_zero = InputPiece::Key {
            intention: Some("0".into()),
            input: "0".into(),
            modifiers: BTreeSet::from([InputModifier::Shift]),
        };
        assert_eq!(table.applied("", shifted_zero), "を");
        assert_eq!(table.applied("", InputPiece::character("0")), "わ");
    }

    #[test]
    fn converts_us_kana_keys() {
        let table = InputTable::default_kana_us();

        assert_eq!(type_text(&table, "qwerty"), "たていすかん");
        assert_eq!(type_text(&table, "f「"), "ば");
        assert_eq!(type_text(&table, "f＝"), "ぱ");
    }

    #[test]
    fn lists_complete_katakana_outputs_for_an_unstable_prefix() {
        let table = InputTable::default_roman_to_kana();
        let values = table.possible_nexts("ky");
        assert!(values.contains(&"キャ".to_owned()));
        assert!(values.contains(&"キュ".to_owned()));
        assert!(values.contains(&"キョ".to_owned()));
        assert!(!values.contains(&"キ".to_owned()));
    }
}
