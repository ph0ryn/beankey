use unicode_segmentation::UnicodeSegmentation;

use crate::lattice::{includes_meaning, prediction_usable};
use crate::{Candidate, ComposingCount, DictionaryEntry, DictionaryError, DictionaryStore};

const BOS_CLASS_ID: usize = 0;
const BOS_MEANING_ID: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub enum PostPredictionKind {
    Additional {
        entries: Vec<DictionaryEntry>,
    },
    Replacement {
        target_entries: Vec<DictionaryEntry>,
        replacement_entries: Vec<DictionaryEntry>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostCompositionPrediction {
    pub text: String,
    pub value: f32,
    pub kind: PostPredictionKind,
    pub is_terminal: bool,
}

impl PostCompositionPrediction {
    fn new(text: String, value: f32, kind: PostPredictionKind) -> Self {
        let is_terminal = matches!(text.as_str(), "。" | "." | "．");
        Self {
            text,
            value,
            kind,
            is_terminal,
        }
    }

    pub fn join(&self, candidate: &Candidate) -> Candidate {
        let mut entries = candidate.entries.clone();
        match &self.kind {
            PostPredictionKind::Additional {
                entries: additional,
            } => {
                entries.extend(additional.iter().cloned());
            }
            PostPredictionKind::Replacement {
                target_entries,
                replacement_entries,
            } => {
                entries.truncate(entries.len().saturating_sub(target_entries.len()));
                entries.extend(replacement_entries.iter().cloned());
            }
        }
        let text = entries.iter().map(|entry| entry.word.as_str()).collect();
        let ruby_count = entries
            .iter()
            .map(|entry| UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count())
            .sum();
        let last_meaning_id = entries
            .iter()
            .rev()
            .find(|entry| includes_meaning(entry))
            .map_or(BOS_MEANING_ID as u16, |entry| entry.meaning_id);
        Candidate::single(
            text,
            self.value,
            ComposingCount::Surface(ruby_count),
            last_meaning_id,
            entries,
        )
    }
}

pub struct PostCompositionPredictor<'a> {
    dictionary: &'a DictionaryStore,
}

impl<'a> PostCompositionPredictor<'a> {
    pub fn new(dictionary: &'a DictionaryStore) -> Self {
        Self { dictionary }
    }

    pub fn predict(
        &self,
        candidate: &Candidate,
    ) -> Result<Vec<PostCompositionPrediction>, DictionaryError> {
        let mut particle_count = 0;
        let zero_hints: Vec<_> = unique(self.zero_hints(candidate, 15)?)
            .into_iter()
            .filter(|prediction| match &prediction.kind {
                PostPredictionKind::Additional { entries }
                    if entries
                        .last()
                        .is_some_and(|entry| (147..=368).contains(&entry.right_id)) =>
                {
                    if particle_count == 3 {
                        false
                    } else {
                        particle_count += 1;
                        true
                    }
                }
                _ => true,
            })
            .collect();
        let replacements = self.replacements(candidate, 15)?;
        let replacement_count = 5_usize.max(10_usize.saturating_sub(zero_hints.len()));
        let mut output = unique(replacements)
            .into_iter()
            .take(replacement_count)
            .collect::<Vec<_>>();
        let seen: std::collections::HashSet<_> =
            output.iter().map(|item| item.text.clone()).collect();
        output.extend(
            unique(zero_hints)
                .into_iter()
                .filter(|item| !seen.contains(&item.text))
                .take(10_usize.saturating_sub(output.len())),
        );
        Ok(output)
    }

    fn zero_hints(
        &self,
        candidate: &Candidate,
        n_best: usize,
    ) -> Result<Vec<PostCompositionPrediction>, DictionaryError> {
        let Some(last) = candidate.entries.last() else {
            return Ok(Vec::new());
        };
        let mut output = Vec::new();
        for entry in self.dictionary.zero_hint_entries(last.right_id)? {
            let meaning = if includes_meaning(&entry) {
                self.dictionary
                    .meaning_cost(
                        usize::from(candidate.last_meaning_id),
                        usize::from(entry.meaning_id),
                    )
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let value = candidate.value
                + self
                    .dictionary
                    .connection_cost(usize::from(last.right_id), usize::from(entry.left_id))?
                + meaning
                + entry.value();
            output.push(PostCompositionPrediction::new(
                entry.word.clone(),
                value,
                PostPredictionKind::Additional {
                    entries: vec![entry],
                },
            ));
        }
        output.sort_by(|left, right| right.value.total_cmp(&left.value));
        output.truncate(n_best);
        Ok(output)
    }

    fn replacements(
        &self,
        candidate: &Candidate,
        n_best: usize,
    ) -> Result<Vec<PostCompositionPrediction>, DictionaryError> {
        let mut prefix_entries = candidate.entries.clone();
        let mut prefix_value = candidate.value;
        let mut prefix_meaning = candidate.last_meaning_id;
        let mut total_word = String::new();
        let mut total_ruby = String::new();
        let mut target_entries = Vec::new();
        let mut output = Vec::new();
        for _ in 0..candidate.entries.len().min(3) {
            let Some(entry) = prefix_entries.pop() else {
                break;
            };
            prefix_value -= entry.value();
            let prior_right = prefix_entries
                .last()
                .map_or(BOS_CLASS_ID, |entry| usize::from(entry.right_id));
            prefix_value -= self
                .dictionary
                .connection_cost(prior_right, usize::from(entry.left_id))?;
            if includes_meaning(&entry) {
                prefix_meaning = prefix_entries
                    .iter()
                    .rev()
                    .find(|entry| includes_meaning(entry))
                    .map_or(BOS_MEANING_ID as u16, |entry| entry.meaning_id);
                prefix_value -= self
                    .dictionary
                    .meaning_cost(usize::from(prefix_meaning), usize::from(entry.meaning_id))
                    .unwrap_or(0.0);
            }
            total_word.insert_str(0, &entry.word);
            total_ruby.insert_str(0, &entry.ruby);
            target_entries.insert(0, entry);
            let ruby_count = UnicodeSegmentation::graphemes(total_ruby.as_str(), true).count();
            let maximum_depth = match ruby_count {
                1 => 3,
                2 => 5,
                _ => usize::MAX,
            };
            for replacement in self
                .dictionary
                .entries_after_prefix(&total_ruby, maximum_depth, 700)?
                .into_iter()
                .filter(|entry| {
                    prediction_usable(entry.right_id) && entry.word.starts_with(&total_word)
                })
            {
                let meaning = if includes_meaning(&replacement) {
                    self.dictionary
                        .meaning_cost(
                            usize::from(prefix_meaning),
                            usize::from(replacement.meaning_id),
                        )
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let value = prefix_value
                    + self
                        .dictionary
                        .connection_cost(prior_right, usize::from(replacement.left_id))?
                    + meaning
                    + replacement.value();
                let suffix = replacement
                    .word
                    .chars()
                    .skip(total_word.chars().count())
                    .collect();
                output.push(PostCompositionPrediction::new(
                    suffix,
                    value,
                    PostPredictionKind::Replacement {
                        target_entries: target_entries.clone(),
                        replacement_entries: vec![replacement],
                    },
                ));
            }
        }
        output.sort_by(|left, right| right.value.total_cmp(&left.value));
        output.truncate(n_best);
        Ok(output)
    }
}

fn unique(values: Vec<PostCompositionPrediction>) -> Vec<PostCompositionPrediction> {
    let mut output: Vec<PostCompositionPrediction> = Vec::new();
    let mut indices: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for value in values.into_iter().filter(|value| !value.text.is_empty()) {
        if let Some(&index) = indices.get(&value.text) {
            if output[index].value < value.value {
                output[index] = value;
            }
        } else {
            indices.insert(value.text.clone(), output.len());
            output.push(value);
        }
    }
    output.sort_by(|left, right| right.value.total_cmp(&left.value));
    output
}
