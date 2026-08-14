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
const PREDICTION_UNUSABLE_RIGHT_IDS: &[u16] = &[
    13, 14, 15, 16, 17, 18, 25, 26, 27, 28, 33, 34, 40, 41, 42, 46, 47, 50, 56, 57, 58, 59, 60, 61,
    62, 63, 64, 74, 75, 76, 77, 78, 79, 86, 87, 88, 93, 94, 95, 99, 100, 103, 107, 108, 109, 110,
    111, 112, 119, 120, 121, 122, 127, 128, 134, 135, 136, 140, 141, 144, 369, 372, 373, 377, 378,
    379, 380, 381, 382, 389, 390, 391, 392, 397, 398, 401, 402, 404, 405, 406, 408, 410, 411, 412,
    413, 416, 417, 418, 419, 420, 421, 426, 427, 431, 433, 434, 437, 438, 441, 442, 443, 447, 448,
    450, 452, 455, 457, 462, 463, 464, 470, 471, 472, 476, 477, 480, 483, 489, 490, 493, 494, 495,
    496, 504, 527, 528, 533, 534, 537, 540, 542, 548, 551, 553, 561, 562, 564, 565, 566, 567, 569,
    571, 572, 574, 575, 576, 577, 579, 581, 582, 585, 587, 589, 590, 591, 594, 595, 596, 597, 598,
    600, 601, 603, 604, 606, 609, 611, 614, 617, 618, 620, 621, 622, 624, 626, 627, 629, 630, 631,
    634, 636, 638, 641, 642, 644, 647, 648, 650, 653, 654, 656, 659, 660, 662, 665, 666, 668, 671,
    672, 673, 675, 676, 677, 678, 681, 682, 683, 684, 687, 688, 691, 692, 693, 694, 697, 698, 699,
    700, 703, 704, 707, 708, 709, 710, 713, 714, 715, 716, 721, 722, 724, 725, 727, 729, 730, 732,
    733, 736, 737, 739, 740, 742, 744, 745, 747, 748, 750, 752, 753, 755, 756, 758, 760, 761, 763,
    764, 766, 768, 769, 770, 771, 774, 775, 776, 777, 778, 779, 780, 781, 782, 783, 786, 787, 790,
    791, 793, 794, 795, 798, 800, 801, 804, 805, 806, 807, 810, 811, 814, 815, 816, 820, 821, 822,
    823, 824, 825, 829, 830, 831, 835, 837, 840, 842, 845, 847, 850, 852, 855, 859, 860, 862, 865,
    866, 868, 869, 871, 872, 873, 875, 877, 878, 880, 881, 884, 885, 887, 888, 889, 890, 891, 893,
    895, 896, 898, 899, 900, 901, 903, 905, 906, 908, 909, 910, 911, 913, 915, 916, 917, 918, 921,
    922, 923, 924, 925, 928, 929, 931, 932, 934, 935, 936, 939, 941, 943, 944, 945, 946, 947, 948,
    949, 950, 951, 952, 958, 959, 960, 961, 962, 963, 964, 965, 966, 967, 973, 974, 975, 976, 977,
    983, 984, 985, 986, 987, 988, 989, 990, 995, 996, 997, 998, 999, 1000, 1001, 1002, 1007, 1008,
    1009, 1010, 1015, 1016, 1017, 1018, 1021, 1022, 1023, 1024, 1029, 1030, 1031, 1032, 1033, 1034,
    1035, 1036, 1041, 1042, 1043, 1044, 1045, 1046, 1047, 1048, 1057, 1058, 1060, 1061, 1063, 1065,
    1066, 1067, 1068, 1069, 1070, 1071, 1072, 1073, 1074, 1075, 1076, 1077, 1078, 1079, 1080, 1081,
    1082, 1083, 1084, 1085, 1086, 1087, 1088, 1089, 1090, 1104, 1105, 1106, 1107, 1108, 1109, 1110,
    1111, 1112, 1113, 1114, 1115, 1116, 1117, 1118, 1119, 1120, 1121, 1122, 1123, 1124, 1125, 1126,
    1127, 1128, 1129, 1130, 1131, 1132, 1133, 1134, 1135, 1136, 1137, 1138, 1139, 1140, 1141, 1142,
    1143, 1144, 1145, 1146, 1147, 1148, 1149, 1150, 1151, 1152, 1153, 1154, 1155, 1156, 1157, 1158,
    1159, 1160, 1161, 1162, 1163, 1164, 1165, 1166, 1167, 1168, 1182, 1183, 1184, 1185, 1186, 1187,
    1188, 1189, 1190, 1191, 1192, 1193, 1194, 1208, 1209, 1210, 1211, 1212, 1213, 1214, 1215, 1220,
    1221, 1222, 1223, 1224, 1225, 1226, 1227, 1228, 1229, 1230, 1231, 1240, 1241, 1242, 1243, 1248,
    1249, 1250, 1251, 1256, 1257, 1258, 1259, 1260, 1261, 1262, 1263, 1268, 1269, 1270, 1271, 1276,
    1278,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversionContext {
    pub right_id: u16,
    pub meaning_id: u16,
}

impl Default for ConversionContext {
    fn default() -> Self {
        Self {
            right_id: BOS_CLASS_ID as u16,
            meaning_id: BOS_MEANING_ID as u16,
        }
    }
}

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
    pub is_learning_target: bool,
    first_clause: Option<FirstClause>,
}

impl Candidate {
    pub fn first_clause_candidate(&self) -> Option<Self> {
        let first = self.first_clause.as_ref()?;
        if first.entry_end + 1 == self.entries.len() {
            return None;
        }
        let entries = self.entries[..=first.entry_end].to_vec();
        let ruby_count = entries
            .iter()
            .map(|entry| UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count())
            .sum();
        Some(Self {
            text: first.text.clone(),
            value: first.value,
            composing_count: first
                .ranges
                .iter()
                .copied()
                .map(LatticeRange::count)
                .reduce(combine_counts)
                .unwrap_or(ComposingCount::Input(0)),
            last_meaning_id: u16::try_from(first.meaning_id).unwrap_or(u16::MAX),
            entries,
            ruby_count,
            is_learning_target: self.is_learning_target,
            first_clause: None,
        })
    }

    fn prediction(
        text: String,
        value: f32,
        composing_count: ComposingCount,
        last_meaning_id: u16,
        entries: Vec<DictionaryEntry>,
    ) -> Self {
        Self::single(text, value, composing_count, last_meaning_id, entries)
    }

    pub(crate) fn single(
        text: String,
        value: f32,
        composing_count: ComposingCount,
        last_meaning_id: u16,
        entries: Vec<DictionaryEntry>,
    ) -> Self {
        let ruby_count = entries
            .iter()
            .map(|entry| UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count())
            .sum();
        Self {
            text,
            value,
            composing_count,
            last_meaning_id,
            entries,
            ruby_count,
            is_learning_target: true,
            first_clause: None,
        }
    }

    pub(crate) fn with_learning_target(mut self, value: bool) -> Self {
        self.is_learning_target = value;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
struct FirstClause {
    text: String,
    value: f32,
    meaning_id: usize,
    ranges: Vec<LatticeRange>,
    entry_end: usize,
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
        self.convert_with_context(composing, tables, n_best, ConversionContext::default())
    }

    pub fn convert_with_context(
        &self,
        composing: &ComposingText,
        tables: &InputTableRegistry,
        n_best: usize,
        context: ConversionContext,
    ) -> Result<Vec<Candidate>, DictionaryError> {
        self.convert_with_entries(composing, tables, n_best, context, &[])
    }

    pub(crate) fn convert_with_entries(
        &self,
        composing: &ComposingText,
        tables: &InputTableRegistry,
        n_best: usize,
        context: ConversionContext,
        additional_entries: &[DictionaryEntry],
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
            for entry in additional_entries {
                let ruby_count = UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count();
                if ruby_count == 0
                    || ruby_count > MAXIMUM_DICTIONARY_LENGTH
                    || ruby_count > surface_count - start
                    || !suffix.starts_with(&entry.ruby)
                    || should_remove(entry)
                {
                    continue;
                }
                let node_index = nodes.len();
                nodes.push(LatticeNode {
                    entry: entry.clone(),
                    range: LatticeRange::Surface {
                        from: start,
                        to: start + ruby_count,
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
                        self.dictionary.connection_cost(
                            usize::from(context.right_id),
                            usize::from(entry.left_id),
                        )?
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
            .map(|path| self.make_candidate(path, context))
            .collect();
        candidates.sort_by(|left, right| right.value.total_cmp(&left.value));
        Ok(unique_candidates(candidates))
    }

    pub fn predict(
        &self,
        composing: &ComposingText,
        tables: &InputTableRegistry,
        n_best: usize,
        context: ConversionContext,
    ) -> Result<Vec<Candidate>, DictionaryError> {
        self.predict_with_entries(composing, tables, n_best, context, &[])
    }

    pub(crate) fn predict_with_entries(
        &self,
        composing: &ComposingText,
        tables: &InputTableRegistry,
        n_best: usize,
        context: ConversionContext,
        additional_entries: &[DictionaryEntry],
    ) -> Result<Vec<Candidate>, DictionaryError> {
        if n_best == 0 || composing.is_empty() {
            return Ok(Vec::new());
        }
        let input_style = composing
            .input()
            .last()
            .map(|element| &element.input_style)
            .unwrap_or(&crate::InputStyle::Direct);
        let surface = composing.surface();
        let prefixes = if matches!(input_style, crate::InputStyle::Direct) {
            vec![to_katakana(&surface)]
        } else {
            let roman_count = surface
                .chars()
                .rev()
                .take_while(|character| character.is_ascii_alphabetic())
                .count();
            if roman_count == 0 {
                vec![to_katakana(&surface)]
            } else {
                let split = surface.len() - roman_count;
                let base = surface[..split].to_owned();
                if base.is_empty() {
                    return Ok(Vec::new());
                }
                let roman = &surface[split..];
                tables
                    .resolve(input_style)
                    .map(|table| {
                        table
                            .possible_nexts(roman)
                            .into_iter()
                            .map(|suffix| format!("{}{suffix}", to_katakana(&base)))
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        let consumed = UnicodeSegmentation::graphemes(surface.as_str(), true).count();
        let mut output = Vec::new();
        for prefix in prefixes {
            let prefix_count = UnicodeSegmentation::graphemes(prefix.as_str(), true).count();
            let maximum_depth = match prefix_count {
                1 => 3,
                2 => 5,
                _ => usize::MAX,
            };
            for entry in self
                .dictionary
                .entries_after_prefix(&prefix, maximum_depth, 700)?
            {
                if PREDICTION_UNUSABLE_RIGHT_IDS
                    .binary_search(&entry.right_id)
                    .is_ok()
                {
                    continue;
                }
                let entry_ruby_count =
                    UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count();
                let penalty = -(entry_ruby_count.saturating_sub(prefix_count) as f32);
                let meaning = if includes_meaning(&entry) {
                    self.dictionary
                        .meaning_cost(
                            usize::from(context.meaning_id),
                            usize::from(entry.meaning_id),
                        )
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let value = self
                    .dictionary
                    .connection_cost(usize::from(context.right_id), usize::from(entry.left_id))?
                    + entry.value()
                    + meaning
                    + penalty;
                let last_meaning_id = if includes_meaning(&entry) {
                    entry.meaning_id
                } else {
                    context.meaning_id
                };
                output.push(Candidate::prediction(
                    entry.word.clone(),
                    value,
                    ComposingCount::Surface(consumed),
                    last_meaning_id,
                    vec![entry],
                ));
            }
            for entry in additional_entries
                .iter()
                .filter(|entry| entry.ruby.starts_with(&prefix) && entry.ruby != prefix)
                .cloned()
            {
                let entry_ruby_count =
                    UnicodeSegmentation::graphemes(entry.ruby.as_str(), true).count();
                let penalty = -(entry_ruby_count.saturating_sub(prefix_count) as f32);
                let meaning = if includes_meaning(&entry) {
                    self.dictionary
                        .meaning_cost(
                            usize::from(context.meaning_id),
                            usize::from(entry.meaning_id),
                        )
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let value = self
                    .dictionary
                    .connection_cost(usize::from(context.right_id), usize::from(entry.left_id))?
                    + entry.value()
                    + meaning
                    + penalty;
                let last_meaning_id = if includes_meaning(&entry) {
                    entry.meaning_id
                } else {
                    context.meaning_id
                };
                output.push(Candidate::prediction(
                    entry.word.clone(),
                    value,
                    ComposingCount::Surface(consumed),
                    last_meaning_id,
                    vec![entry],
                ));
            }
        }
        output.sort_by(|left, right| right.value.total_cmp(&left.value));
        output = unique_candidates(output);
        output.truncate(n_best);
        Ok(output)
    }

    pub fn word_candidates(
        &self,
        composing: &ComposingText,
    ) -> Result<Vec<Candidate>, DictionaryError> {
        if composing.is_empty() {
            return Ok(Vec::new());
        }
        let katakana = to_katakana(&composing.surface());
        let mut output = Vec::new();
        for matched in self
            .dictionary
            .matches_from_start(&katakana, MAXIMUM_DICTIONARY_LENGTH)?
        {
            for entry in matched.entries {
                let value = entry.value();
                output.push(Candidate::single(
                    entry.word.clone(),
                    value,
                    ComposingCount::Surface(matched.surface_end),
                    entry.meaning_id,
                    vec![entry],
                ));
            }
        }
        output = unique_candidates(output);
        output.sort_by(|left, right| {
            right
                .ruby_count
                .cmp(&left.ruby_count)
                .then_with(|| right.value.total_cmp(&left.value))
        });
        Ok(output)
    }

    pub fn representation_candidates(
        &self,
        composing: &ComposingText,
        full_width_roman: bool,
        half_width_kana: bool,
    ) -> Vec<Candidate> {
        let katakana = to_katakana(&composing.surface());
        let composing_count = ComposingCount::Input(composing.input().len());
        let mut output = Vec::new();
        let katakana_value = -14.0 * katakana_score(&katakana);
        output.push(representation_candidate(
            katakana.clone(),
            katakana.clone(),
            katakana_value,
            composing_count.clone(),
        ));
        output.push(representation_candidate(
            to_hiragana(&katakana),
            katakana.clone(),
            -14.5,
            composing_count.clone(),
        ));
        output.push(representation_candidate(
            katakana.to_uppercase(),
            katakana.clone(),
            -14.6,
            composing_count.clone(),
        ));
        if full_width_roman {
            output.push(representation_candidate(
                to_full_width(&katakana),
                katakana.clone(),
                -14.7,
                composing_count.clone(),
            ));
        }
        if half_width_kana {
            output.push(representation_candidate(
                to_half_width(&katakana),
                katakana.clone(),
                -15.0,
                composing_count,
            ));
        }
        output
    }

    fn make_candidate(&self, path: Arc<RegisteredPath>, context: ConversionContext) -> Candidate {
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
            meaning_id: usize::from(context.meaning_id),
            ranges: Vec::new(),
            entry_end: 0,
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
                    entry_end: entries.len(),
                });
            }
            let clause = clauses.last_mut().expect("a candidate has a clause");
            clause.text.push_str(&entry.word);
            clause.value = current.total;
            clause.ranges.push(current.range);
            clause.entry_end = entries.len();
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
        let first_clause = clauses.first().map(|clause| FirstClause {
            text: clause.text.clone(),
            value: clause.value,
            meaning_id: clause.meaning_id,
            ranges: clause.ranges.clone(),
            entry_end: clause.entry_end,
        });
        Candidate {
            text,
            value: clauses
                .last()
                .map_or(meaning_value, |clause| clause.value + meaning_value),
            composing_count,
            last_meaning_id: u16::try_from(previous_meaning).unwrap_or(u16::MAX),
            entries,
            ruby_count,
            is_learning_target: true,
            first_clause,
        }
    }
}

struct Clause {
    text: String,
    value: f32,
    meaning_id: usize,
    ranges: Vec<LatticeRange>,
    entry_end: usize,
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

pub(crate) fn is_clause(former: usize, latter: usize) -> bool {
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

pub(crate) fn includes_meaning(entry: &DictionaryEntry) -> bool {
    let left = usize::from(entry.left_id);
    let right = usize::from(entry.right_id);
    (895..=1280).contains(&left)
        || (895..=1280).contains(&right)
        || (1297..=1305).contains(&left)
        || (1297..=1305).contains(&right)
        || word_type(left) == 1
        || word_type(right) == 1
}

pub(crate) fn prediction_usable(right_id: u16) -> bool {
    PREDICTION_UNUSABLE_RIGHT_IDS
        .binary_search(&right_id)
        .is_err()
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

fn to_hiragana(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{30A1}'..='\u{30F6}' => {
                char::from_u32(u32::from(character) - 96).expect("hiragana scalar is valid")
            }
            _ => character,
        })
        .collect()
}

fn katakana_score(value: &str) -> f32 {
    value.chars().fold(1.0, |score, character| {
        score
            * if "プヴペィフ".contains(character) {
                0.5
            } else if "ュピポ".contains(character) {
                0.6
            } else if "パォグーム".contains(character) {
                0.7
            } else {
                1.0
            }
    })
}

fn representation_candidate(
    word: String,
    ruby: String,
    value: f32,
    composing_count: ComposingCount,
) -> Candidate {
    let entry = DictionaryEntry {
        word: word.clone(),
        ruby,
        left_id: 1288,
        right_id: 1288,
        meaning_id: 501,
        base_value: value,
        adjustment: 0.0,
        metadata: Default::default(),
    };
    Candidate::single(word, value, composing_count, 501, vec![entry])
}

fn to_full_width(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            ' ' => '\u{3000}',
            '!'..='~' => {
                char::from_u32(u32::from(character) + 0xfee0).expect("full-width scalar is valid")
            }
            _ => character,
        })
        .collect()
}

fn to_half_width(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        let mapped = match character {
            '。' => "｡",
            '「' => "｢",
            '」' => "｣",
            '、' => "､",
            '・' => "･",
            'ヲ' => "ｦ",
            'ァ' => "ｧ",
            'ィ' => "ｨ",
            'ゥ' => "ｩ",
            'ェ' => "ｪ",
            'ォ' => "ｫ",
            'ャ' => "ｬ",
            'ュ' => "ｭ",
            'ョ' => "ｮ",
            'ッ' => "ｯ",
            'ー' => "ｰ",
            'ア' => "ｱ",
            'イ' => "ｲ",
            'ウ' => "ｳ",
            'エ' => "ｴ",
            'オ' => "ｵ",
            'カ' => "ｶ",
            'キ' => "ｷ",
            'ク' => "ｸ",
            'ケ' => "ｹ",
            'コ' => "ｺ",
            'サ' => "ｻ",
            'シ' => "ｼ",
            'ス' => "ｽ",
            'セ' => "ｾ",
            'ソ' => "ｿ",
            'タ' => "ﾀ",
            'チ' => "ﾁ",
            'ツ' => "ﾂ",
            'テ' => "ﾃ",
            'ト' => "ﾄ",
            'ナ' => "ﾅ",
            'ニ' => "ﾆ",
            'ヌ' => "ﾇ",
            'ネ' => "ﾈ",
            'ノ' => "ﾉ",
            'ハ' => "ﾊ",
            'ヒ' => "ﾋ",
            'フ' => "ﾌ",
            'ヘ' => "ﾍ",
            'ホ' => "ﾎ",
            'マ' => "ﾏ",
            'ミ' => "ﾐ",
            'ム' => "ﾑ",
            'メ' => "ﾒ",
            'モ' => "ﾓ",
            'ヤ' => "ﾔ",
            'ユ' => "ﾕ",
            'ヨ' => "ﾖ",
            'ラ' => "ﾗ",
            'リ' => "ﾘ",
            'ル' => "ﾙ",
            'レ' => "ﾚ",
            'ロ' => "ﾛ",
            'ワ' => "ﾜ",
            'ン' => "ﾝ",
            'ヴ' => "ｳﾞ",
            'ガ' => "ｶﾞ",
            'ギ' => "ｷﾞ",
            'グ' => "ｸﾞ",
            'ゲ' => "ｹﾞ",
            'ゴ' => "ｺﾞ",
            'ザ' => "ｻﾞ",
            'ジ' => "ｼﾞ",
            'ズ' => "ｽﾞ",
            'ゼ' => "ｾﾞ",
            'ゾ' => "ｿﾞ",
            'ダ' => "ﾀﾞ",
            'ヂ' => "ﾁﾞ",
            'ヅ' => "ﾂﾞ",
            'デ' => "ﾃﾞ",
            'ド' => "ﾄﾞ",
            'バ' => "ﾊﾞ",
            'ビ' => "ﾋﾞ",
            'ブ' => "ﾌﾞ",
            'ベ' => "ﾍﾞ",
            'ボ' => "ﾎﾞ",
            'パ' => "ﾊﾟ",
            'ピ' => "ﾋﾟ",
            'プ' => "ﾌﾟ",
            'ペ' => "ﾍﾟ",
            'ポ' => "ﾎﾟ",
            _ => {
                output.push(character);
                continue;
            }
        };
        output.push_str(mapped);
    }
    output
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

    #[test]
    fn converts_optional_width_representations() {
        assert_eq!(to_full_width("ABC 123"), "ＡＢＣ　１２３");
        assert_eq!(to_half_width("ガッツポーズ"), "ｶﾞｯﾂﾎﾟｰｽﾞ");
    }
}
