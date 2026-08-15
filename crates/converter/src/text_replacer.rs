use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use unicode_segmentation::UnicodeSegmentation;

use crate::kana::to_hiragana;

#[derive(Debug)]
pub enum TextReplacerError {
    Io { path: PathBuf, source: io::Error },
    MalformedLine { line: usize, field_count: usize },
}

impl fmt::Display for TextReplacerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::MalformedLine { line, field_count } => write!(
                formatter,
                "emoji dictionary line {line} has {field_count} fields instead of 3"
            ),
        }
    }
}

impl Error for TextReplacerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::MalformedLine { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TextSearchResult {
    pub query: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacementCandidate {
    pub target: String,
    pub replacement: String,
    pub base: String,
}

impl ReplacementCandidate {
    pub fn text(&self) -> &str {
        &self.replacement
    }
}

#[derive(Clone, Debug)]
struct EmojiGroup {
    base: String,
    variations: Vec<String>,
}

impl EmojiGroup {
    fn contains(&self, emoji: &str) -> bool {
        self.base == emoji || self.variations.iter().any(|item| item == emoji)
    }

    fn alternatives(&self, emoji: &str) -> impl Iterator<Item = &str> {
        std::iter::once(self.base.as_str())
            .chain(self.variations.iter().map(String::as_str))
            .filter(move |item| *item != emoji)
    }
}

#[derive(Clone, Debug, Default)]
pub struct TextReplacer {
    emoji_search: HashMap<String, Vec<String>>,
    emoji_groups: Vec<EmojiGroup>,
    non_base_emojis: HashSet<String>,
}

impl TextReplacer {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TextReplacerError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|source| TextReplacerError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&contents)
    }

    pub fn is_empty(&self) -> bool {
        self.emoji_search.is_empty()
            && self.emoji_groups.is_empty()
            && self.non_base_emojis.is_empty()
    }

    pub fn search(&self, query: &str, ignore_non_base_emoji: bool) -> Vec<TextSearchResult> {
        let query = normalize_query(query);
        self.emoji_search
            .get(&query)
            .into_iter()
            .flatten()
            .filter(|emoji| !ignore_non_base_emoji || !self.non_base_emojis.contains(*emoji))
            .map(|emoji| TextSearchResult {
                query: query.clone(),
                text: emoji.clone(),
            })
            .collect()
    }

    pub fn replacements(
        &self,
        left: &str,
        center: &str,
        _right: &str,
    ) -> Vec<ReplacementCandidate> {
        let center_graphemes = UnicodeSegmentation::graphemes(center, true).collect::<Vec<_>>();
        let target = if center_graphemes.len() == 1 {
            Some(center_graphemes[0])
        } else {
            UnicodeSegmentation::graphemes(left, true).next_back()
        };
        let Some(target) = target else {
            return Vec::new();
        };
        let Some(group) = self
            .emoji_groups
            .iter()
            .find(|group| group.contains(target))
        else {
            return Vec::new();
        };
        group
            .alternatives(target)
            .map(|replacement| ReplacementCandidate {
                target: target.to_owned(),
                replacement: replacement.to_owned(),
                base: group.base.clone(),
            })
            .collect()
    }

    fn parse(contents: &str) -> Result<Self, TextReplacerError> {
        let mut replacer = Self::default();
        for (index, line) in contents.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(TextReplacerError::MalformedLine {
                    line: index + 1,
                    field_count: fields.len(),
                });
            }
            let base = fields[0].to_owned();
            let variations = split_list(fields[2]);
            for query in split_list(fields[1]) {
                let values = replacer.emoji_search.entry(query).or_default();
                values.push(base.clone());
                values.extend(variations.iter().cloned());
            }
            replacer.non_base_emojis.extend(variations.iter().cloned());
            replacer.emoji_groups.push(EmojiGroup { base, variations });
        }
        Ok(replacer)
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn normalize_query(value: &str) -> String {
    to_hiragana(&value.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMOJI_DATA: &str = "👍️\tgood,いいね\t👍🏻️,👍🏿️\n😀️\tえがお,笑顔\t\n";

    #[test]
    fn searches_normalized_queries_and_can_hide_variations() {
        let replacer = TextReplacer::parse(EMOJI_DATA).unwrap();

        let all = replacer.search("GOOD", false);
        assert_eq!(
            all.iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["👍️", "👍🏻️", "👍🏿️"]
        );
        assert_eq!(replacer.search("イイネ", true)[0].text, "👍️");
        assert!(replacer.search("unknown", false).is_empty());
    }

    #[test]
    fn replaces_a_selected_or_immediately_preceding_emoji() {
        let replacer = TextReplacer::parse(EMOJI_DATA).unwrap();

        let selected = replacer.replacements("", "👍🏻️", "");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].target, "👍🏻️");
        assert_eq!(selected[0].replacement, "👍️");
        assert_eq!(selected[0].base, "👍️");

        let preceding = replacer.replacements("ok👍🏿️", "thanks", "");
        assert_eq!(preceding[0].target, "👍🏿️");
        assert_eq!(preceding[0].replacement, "👍️");
    }

    #[test]
    fn rejects_malformed_data_without_partially_loading_it() {
        assert!(matches!(
            TextReplacer::parse("😀\tえがお"),
            Err(TextReplacerError::MalformedLine {
                line: 1,
                field_count: 2
            })
        ));
    }
}
