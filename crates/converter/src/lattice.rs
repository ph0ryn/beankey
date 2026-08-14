use std::collections::HashMap;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    ComposingCount, ComposingText, DictionaryEntry, DictionaryError, DictionaryStore,
    InputTableRegistry,
};

const BOS_CLASS_ID: usize = 0;
const BOS_MEANING_ID: usize = 500;
const DICTIONARY_THRESHOLD: f32 = -17.0;
const MAXIMUM_DICTIONARY_LENGTH: usize = 20;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum LatticeIndex {
    Input(usize),
    Surface(usize),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LatticeRange {
    Input { from: usize, to: usize },
    Surface { from: usize, to: usize },
}

impl LatticeRange {
    fn end(self) -> LatticeIndex {
        match self {
            Self::Input { to, .. } => LatticeIndex::Input(to),
            Self::Surface { to, .. } => LatticeIndex::Surface(to),
        }
    }

    fn count(self) -> ComposingCount {
        match self {
            Self::Input { from, to } => ComposingCount::Input(to - from),
            Self::Surface { from, to } => ComposingCount::Surface(to - from),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DualIndex {
    Input(usize),
    Surface(usize),
    Both { input: usize, surface: usize },
}

impl DualIndex {
    fn input(self) -> Option<usize> {
        match self {
            Self::Input(index) | Self::Both { input: index, .. } => Some(index),
            Self::Surface(_) => None,
        }
    }

    fn surface(self) -> Option<usize> {
        match self {
            Self::Surface(index) | Self::Both { surface: index, .. } => Some(index),
            Self::Input(_) => None,
        }
    }
}

struct DualIndexMap {
    input_to_surface: HashMap<usize, usize>,
    surface_to_input: HashMap<usize, usize>,
    identity: bool,
}

impl DualIndexMap {
    fn new(composing: &ComposingText, tables: &InputTableRegistry) -> Self {
        let input_to_surface = composing.input_to_surface_map(tables);
        let identity = composing.input().len() == composing.surface_graphemes().len()
            && (0..=composing.input().len())
                .all(|index| input_to_surface.get(&index) == Some(&index));
        let mut surface_to_input = HashMap::with_capacity(input_to_surface.len());
        for (&input, &surface) in &input_to_surface {
            surface_to_input.entry(surface).or_insert(input);
        }
        Self {
            input_to_surface,
            surface_to_input,
            identity,
        }
    }

    fn dual(&self, index: LatticeIndex) -> DualIndex {
        if self.identity {
            let index = match index {
                LatticeIndex::Input(index) | LatticeIndex::Surface(index) => index,
            };
            return DualIndex::Both {
                input: index,
                surface: index,
            };
        }
        match index {
            LatticeIndex::Input(input) => match self.input_to_surface.get(&input) {
                Some(&surface) => DualIndex::Both { input, surface },
                None => DualIndex::Input(input),
            },
            LatticeIndex::Surface(surface) => match self.surface_to_input.get(&surface) {
                Some(&input) => DualIndex::Both { input, surface },
                None => DualIndex::Surface(surface),
            },
        }
    }

    fn indices(&self, input_count: usize, surface_count: usize) -> Vec<DualIndex> {
        if self.identity {
            return (0..input_count.min(surface_count))
                .map(|index| DualIndex::Both {
                    input: index,
                    surface: index,
                })
                .collect();
        }
        let mut indices = Vec::with_capacity(input_count + surface_count);
        let mut surface_pointer = 0;
        for input in 0..input_count {
            if let Some(&surface) = self.input_to_surface.get(&input) {
                for value in surface_pointer.min(surface)..surface {
                    indices.push(DualIndex::Surface(value));
                }
                if surface_pointer <= surface && surface < surface_count {
                    indices.push(DualIndex::Both { input, surface });
                } else {
                    indices.push(DualIndex::Input(input));
                }
                surface_pointer = surface + 1;
            } else {
                indices.push(DualIndex::Input(input));
            }
        }
        for surface in surface_pointer.min(surface_count)..surface_count {
            indices.push(DualIndex::Surface(surface));
        }
        indices
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub text: String,
    pub value: f32,
    pub composing_count: ComposingCount,
    pub last_meaning_id: u16,
    pub entries: Vec<DictionaryEntry>,
    pub ruby_count: usize,
}

#[derive(Clone)]
struct RegisteredPath {
    entry: DictionaryEntry,
    range: LatticeRange,
    previous: Option<Arc<Self>>,
    total: f32,
}

#[derive(Clone)]
struct Predecessor {
    path: Option<Arc<RegisteredPath>>,
    total: f32,
}

struct LatticeNode {
    entry: DictionaryEntry,
    range: LatticeRange,
    predecessors: Vec<Predecessor>,
}

pub struct NormalConverter<'a> {
    dictionary: &'a DictionaryStore,
}

impl<'a> NormalConverter<'a> {
    pub fn new(dictionary: &'a DictionaryStore) -> Self {
        Self { dictionary }
    }

    pub fn convert(
        &self,
        composing: &ComposingText,
        tables: &InputTableRegistry,
        n_best: usize,
    ) -> Result<Vec<Candidate>, DictionaryError> {
        if n_best == 0 || composing.is_empty() {
            return Ok(Vec::new());
        }

        let surface_count = composing.surface_graphemes().len();
        let index_map = DualIndexMap::new(composing, tables);
        let indices = index_map.indices(composing.input().len(), surface_count);
        let katakana = to_katakana(&composing.surface());
        let katakana_graphemes: Vec<_> = UnicodeSegmentation::graphemes(katakana.as_str(), true)
            .map(str::to_owned)
            .collect();

        let mut nodes = Vec::new();
        let mut surface_nodes = vec![Vec::new(); surface_count];
        for start in 0..surface_count {
            let suffix = katakana_graphemes[start..].concat();
            for matched in self
                .dictionary
                .matches_from_start(&suffix, MAXIMUM_DICTIONARY_LENGTH)?
            {
                for entry in matched.entries {
                    if should_remove(&entry) {
                        continue;
                    }
                    let node_index = nodes.len();
                    nodes.push(LatticeNode {
                        entry,
                        range: LatticeRange::Surface {
                            from: start,
                            to: start + matched.surface_end,
                        },
                        predecessors: if start == 0 {
                            vec![Predecessor {
                                path: None,
                                total: 0.0,
                            }]
                        } else {
                            Vec::new()
                        },
                    });
                    surface_nodes[start].push(node_index);
                }
            }
        }

        let mut results = Vec::new();
        for index in indices {
            let Some(surface_index) = index.surface() else {
                continue;
            };
            let is_head = index.input() == Some(0) && surface_index == 0;
            let current_nodes = surface_nodes[surface_index].clone();
            for node_index in current_nodes {
                let (entry, range, predecessors) = {
                    let node = &nodes[node_index];
                    (node.entry.clone(), node.range, node.predecessors.clone())
                };
                if predecessors.is_empty() {
                    continue;
                }
                let mut completed = Vec::with_capacity(predecessors.len());
                for predecessor in predecessors {
                    let head_connection = if is_head {
                        self.dictionary
                            .connection_cost(BOS_CLASS_ID, usize::from(entry.left_id))?
                    } else {
                        0.0
                    };
                    let total = predecessor.total + entry.value() + head_connection;
                    completed.push(Arc::new(RegisteredPath {
                        entry: entry.clone(),
                        range,
                        previous: predecessor.path,
                        total,
                    }));
                }

                let next_index = index_map.dual(range.end());
                if next_index.surface() == Some(surface_count) {
                    results.extend(completed);
                    continue;
                }
                let Some(next_surface) = next_index.surface() else {
                    continue;
                };
                let next_nodes = surface_nodes[next_surface].clone();
                for next_node_index in next_nodes {
                    let connection = self.dictionary.connection_cost(
                        usize::from(entry.right_id),
                        usize::from(nodes[next_node_index].entry.left_id),
                    )?;
                    for path in &completed {
                        insert_predecessor(
                            &mut nodes[next_node_index].predecessors,
                            Predecessor {
                                path: Some(Arc::clone(path)),
                                total: path.total + connection,
                            },
                            n_best,
                        );
                    }
                }
            }
        }

        let mut candidates: Vec<_> = results
            .into_iter()
            .map(|path| self.make_candidate(path))
            .collect();
        candidates.sort_by(|left, right| right.value.total_cmp(&left.value));
        Ok(unique_candidates(candidates))
    }

    fn make_candidate(&self, path: Arc<RegisteredPath>) -> Candidate {
        let mut paths = Vec::new();
        let mut cursor = Some(path);
        while let Some(current) = cursor {
            paths.push(Arc::clone(&current));
            cursor = current.previous.clone();
        }
        paths.reverse();

        let mut clauses = vec![Clause {
            text: String::new(),
            value: 0.0,
            meaning_id: BOS_MEANING_ID,
            ranges: Vec::new(),
        }];
        let mut entries = Vec::with_capacity(paths.len());
        let mut previous_right = BOS_CLASS_ID;
        for current in paths {
            let entry = &current.entry;
            let continue_clause = clauses.last().is_some_and(|clause| clause.text.is_empty())
                || !is_clause(previous_right, usize::from(entry.left_id));
            if !continue_clause {
                clauses.push(Clause {
                    text: String::new(),
                    value: current.total,
                    meaning_id: if includes_meaning(entry) {
                        usize::from(entry.meaning_id)
                    } else {
                        BOS_MEANING_ID
                    },
                    ranges: Vec::new(),
                });
            }
            let clause = clauses.last_mut().expect("a candidate has a clause");
            clause.text.push_str(&entry.word);
            clause.value = current.total;
            clause.ranges.push(current.range);
            if (clause.meaning_id == BOS_MEANING_ID && entry.meaning_id != BOS_MEANING_ID as u16)
                || includes_meaning(entry)
            {
                clause.meaning_id = usize::from(entry.meaning_id);
            }
            previous_right = usize::from(entry.right_id);
            entries.push(entry.clone());
        }

        let mut previous_meaning = BOS_MEANING_ID;
        let mut meaning_value = 0.0;
        for clause in &clauses {
            meaning_value += self
                .dictionary
                .meaning_cost(previous_meaning, clause.meaning_id)
                .unwrap_or(0.0);
            previous_meaning = clause.meaning_id;
        }
        let text = clauses.iter().map(|clause| clause.text.as_str()).collect();
        let composing_count = clauses
            .iter()
            .flat_map(|clause| clause.ranges.iter().copied())
            .map(LatticeRange::count)
            .reduce(combine_counts)
            .unwrap_or(ComposingCount::Input(0));
        let ruby_count = entries
            .iter()
            .map(|entry| UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count())
            .sum();
        Candidate {
            text,
            value: clauses
                .last()
                .map_or(meaning_value, |clause| clause.value + meaning_value),
            composing_count,
            last_meaning_id: u16::try_from(previous_meaning).unwrap_or(u16::MAX),
            entries,
            ruby_count,
        }
    }
}

struct Clause {
    text: String,
    value: f32,
    meaning_id: usize,
    ranges: Vec<LatticeRange>,
}

fn insert_predecessor(values: &mut Vec<Predecessor>, value: Predecessor, n_best: usize) {
    let index = values
        .iter()
        .rposition(|existing| existing.total >= value.total)
        .map_or(0, |index| index + 1);
    if index == n_best {
        return;
    }
    if values.len() >= n_best {
        values.pop();
    }
    values.insert(index, value);
}

fn unique_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut output: Vec<Candidate> = Vec::new();
    let mut indices = HashMap::new();
    for candidate in candidates
        .into_iter()
        .filter(|candidate| !candidate.text.is_empty())
    {
        if let Some(&index) = indices.get(&candidate.text) {
            let existing: &mut Candidate = &mut output[index];
            if existing.value < candidate.value || existing.ruby_count < candidate.ruby_count {
                *existing = candidate;
            }
        } else {
            indices.insert(candidate.text.clone(), output.len());
            output.push(candidate);
        }
    }
    output.sort_by(|left, right| right.value.total_cmp(&left.value));
    output
}

fn combine_counts(left: ComposingCount, right: ComposingCount) -> ComposingCount {
    match (left, right) {
        (ComposingCount::Input(left), ComposingCount::Input(right)) => {
            ComposingCount::Input(left + right)
        }
        (ComposingCount::Surface(left), ComposingCount::Surface(right)) => {
            ComposingCount::Surface(left + right)
        }
        (left, right) => ComposingCount::Composite(Box::new(left), Box::new(right)),
    }
}

fn should_remove(entry: &DictionaryEntry) -> bool {
    let distance = entry.value() - DICTIONARY_THRESHOLD;
    if distance < 0.0 {
        return true;
    }
    let word_count = UnicodeSegmentation::graphemes(entry.word.as_str(), true).count();
    word_count == 0 || -2.0 / (word_count as f32) < -distance
}

fn word_type(class_id: usize) -> u8 {
    if matches!(class_id, 0 | 1316) {
        return 3;
    }
    if matches!(class_id, 1315 | 6 | 557 | 558 | 559 | 560) {
        return 0;
    }
    if (561..868).contains(&class_id)
        || (1283..1297).contains(&class_id)
        || (1306..1310).contains(&class_id)
        || (11..53).contains(&class_id)
        || (555..557).contains(&class_id)
        || (1281..1283).contains(&class_id)
        || matches!(class_id, 1314 | 3 | 2 | 4 | 5 | 1 | 9)
    {
        return 1;
    }
    2
}

fn is_clause(former: usize, latter: usize) -> bool {
    let latter_type = word_type(latter);
    if latter_type == 3 {
        return false;
    }
    let former_type = word_type(former);
    if former_type == 3 {
        return false;
    }
    matches!(latter_type, 0 | 1) && former_type != 0
}

fn includes_meaning(entry: &DictionaryEntry) -> bool {
    let left = usize::from(entry.left_id);
    let right = usize::from(entry.right_id);
    (895..=1280).contains(&left)
        || (895..=1280).contains(&right)
        || (1297..=1305).contains(&left)
        || (1297..=1305).contains(&right)
        || word_type(left) == 1
        || word_type(right) == 1
}

fn to_katakana(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{3041}'..='\u{3096}' => {
                char::from_u32(u32::from(character) + 96).expect("katakana scalar is valid")
            }
            _ => character,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputStyle;

    fn entry(word: &str, value: f32) -> DictionaryEntry {
        DictionaryEntry {
            word: word.into(),
            ruby: "ルビ".into(),
            left_id: 1285,
            right_id: 1285,
            meaning_id: 501,
            base_value: value,
            adjustment: 0.0,
            metadata: Default::default(),
        }
    }

    #[test]
    fn katakana_conversion_matches_the_fixed_upstream_range() {
        assert_eq!(to_katakana("ぁあゖー漢"), "ァアヶー漢");
    }

    #[test]
    fn predecessor_order_is_descending_and_stable_for_ties() {
        let mut values = Vec::new();
        for total in [-2.0, -1.0, -1.0, -3.0, 0.0] {
            insert_predecessor(&mut values, Predecessor { path: None, total }, 4);
        }
        assert_eq!(
            values.iter().map(|value| value.total).collect::<Vec<_>>(),
            vec![0.0, -1.0, -1.0, -2.0]
        );
    }

    #[test]
    fn dictionary_threshold_uses_the_upstream_word_length_penalty() {
        assert!(!should_remove(&entry("長い語", -16.0)));
        assert!(should_remove(&entry("語", -16.0)));
        assert!(should_remove(&entry("語", -18.0)));
    }

    #[test]
    fn dual_indices_keep_roman_input_and_surface_positions_in_order() {
        let tables = InputTableRegistry::new();
        let mut composing = ComposingText::new();
        composing.insert_str("kya", InputStyle::RomanToKana, &tables);
        let map = DualIndexMap::new(&composing, &tables);
        let indices = map.indices(composing.input().len(), composing.surface_graphemes().len());
        assert_eq!(indices.first().and_then(|index| index.input()), Some(0));
        assert_eq!(indices.first().and_then(|index| index.surface()), Some(0));
        assert!(indices.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
