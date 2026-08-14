use std::error::Error;
use std::fmt;

use crate::{
    Candidate, ComposingText, ConversionContext, DictionaryError, InputStyle, InputTableRegistry,
    NormalConverter,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    CandidateOutOfRange {
        index: usize,
        candidate_count: usize,
    },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CandidateOutOfRange {
                index,
                candidate_count,
            } => write!(
                formatter,
                "candidate index {index} is outside the current {candidate_count} candidates"
            ),
        }
    }
}

impl Error for SelectionError {}

#[derive(Clone, Debug, Default)]
pub struct ConversionSession {
    composing: ComposingText,
    candidates: Vec<Candidate>,
    context: ConversionContext,
}

impl ConversionSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn composing(&self) -> &ComposingText {
        &self.composing
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    pub fn insert_str(
        &mut self,
        value: &str,
        input_style: InputStyle,
        tables: &InputTableRegistry,
    ) {
        self.composing.insert_str(value, input_style, tables);
        self.candidates.clear();
    }

    pub fn delete_backward(&mut self, count: usize, tables: &InputTableRegistry) {
        self.composing.delete_backward(count, tables);
        self.candidates.clear();
        if self.composing.is_empty() {
            self.context = ConversionContext::default();
        }
    }

    pub fn delete_forward(&mut self, count: usize, tables: &InputTableRegistry) {
        self.composing.delete_forward(count, tables);
        self.candidates.clear();
        if self.composing.is_empty() {
            self.context = ConversionContext::default();
        }
    }

    pub fn move_cursor(&mut self, count: isize) -> isize {
        let moved = self.composing.move_cursor(count);
        self.candidates.clear();
        moved
    }

    pub fn request_candidates(
        &mut self,
        converter: &NormalConverter<'_>,
        tables: &InputTableRegistry,
        n_best: usize,
    ) -> Result<&[Candidate], DictionaryError> {
        self.candidates =
            converter.convert_with_context(&self.composing, tables, n_best, self.context)?;
        let mut seen: std::collections::HashSet<_> = self
            .candidates
            .iter()
            .map(|candidate| candidate.text.clone())
            .collect();
        let mut first_clauses: Vec<_> = self
            .candidates
            .iter()
            .filter_map(Candidate::first_clause_candidate)
            .filter(|candidate| seen.insert(candidate.text.clone()))
            .collect();
        first_clauses.sort_by(|left, right| {
            right
                .ruby_count
                .cmp(&left.ruby_count)
                .then_with(|| right.value.total_cmp(&left.value))
        });
        first_clauses.truncate(5);
        self.candidates.extend(first_clauses);
        Ok(&self.candidates)
    }

    pub fn request_predictions(
        &self,
        converter: &NormalConverter<'_>,
        tables: &InputTableRegistry,
        n_best: usize,
    ) -> Result<Vec<Candidate>, DictionaryError> {
        converter.predict(&self.composing, tables, n_best, self.context)
    }

    pub fn select_candidate(
        &mut self,
        index: usize,
        tables: &InputTableRegistry,
    ) -> Result<String, SelectionError> {
        let candidate =
            self.candidates
                .get(index)
                .cloned()
                .ok_or(SelectionError::CandidateOutOfRange {
                    index,
                    candidate_count: self.candidates.len(),
                })?;
        self.composing
            .complete_prefix(candidate.composing_count, tables);
        if let Some(last) = candidate.entries.last() {
            self.context = ConversionContext {
                right_id: last.right_id,
                meaning_id: candidate.last_meaning_id,
            };
        }
        self.candidates.clear();
        if self.composing.is_empty() {
            self.composing.stop();
            self.context = ConversionContext::default();
        }
        Ok(candidate.text)
    }

    pub fn reset(&mut self) {
        self.composing.stop();
        self.candidates.clear();
        self.context = ConversionContext::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_invalidates_candidates_and_reset_clears_composition() {
        let tables = InputTableRegistry::new();
        let mut session = ConversionSession::new();
        session.insert_str("kana", InputStyle::RomanToKana, &tables);
        assert_eq!(session.composing().surface(), "かな");
        session.move_cursor(-1);
        session.delete_forward(1, &tables);
        assert_eq!(session.composing().surface(), "か");
        assert!(session.candidates().is_empty());

        session.reset();
        assert!(session.composing().is_empty());
    }

    #[test]
    fn rejects_a_stale_candidate_index_without_changing_input() {
        let tables = InputTableRegistry::new();
        let mut session = ConversionSession::new();
        session.insert_str("かな", InputStyle::Direct, &tables);
        let error = session.select_candidate(0, &tables).unwrap_err();
        assert_eq!(
            error,
            SelectionError::CandidateOutOfRange {
                index: 0,
                candidate_count: 0
            }
        );
        assert_eq!(session.composing().surface(), "かな");
    }
}
