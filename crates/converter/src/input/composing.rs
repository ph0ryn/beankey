use std::collections::HashMap;

use super::table::{Grapheme, graphemes};
use super::{InputPiece, InputTable};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum InputTableId {
    Empty,
    DefaultRomanToKana,
    DefaultAzik,
    DefaultKanaJis,
    DefaultKanaUs,
    Named(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum InputStyle {
    Direct,
    RomanToKana,
    Mapped(InputTableId),
}

impl InputStyle {
    fn frozen() -> Self {
        Self::Mapped(InputTableId::Empty)
    }
}

#[derive(Clone, Debug)]
pub struct InputTableRegistry {
    empty: InputTable,
    roman_to_kana: InputTable,
    azik: InputTable,
    kana_jis: InputTable,
    kana_us: InputTable,
    custom: HashMap<String, InputTable>,
}

impl Default for InputTableRegistry {
    fn default() -> Self {
        Self {
            empty: InputTable::empty(),
            roman_to_kana: InputTable::default_roman_to_kana(),
            azik: InputTable::default_azik(),
            kana_jis: InputTable::default_kana_jis(),
            kana_us: InputTable::default_kana_us(),
            custom: HashMap::new(),
        }
    }
}

impl InputTableRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, table: InputTable) {
        self.custom.insert(name.into(), table);
    }

    pub(crate) fn resolve(&self, style: &InputStyle) -> Option<&InputTable> {
        match style {
            InputStyle::Direct => None,
            InputStyle::RomanToKana => Some(&self.roman_to_kana),
            InputStyle::Mapped(InputTableId::Empty) => Some(&self.empty),
            InputStyle::Mapped(InputTableId::DefaultRomanToKana) => Some(&self.roman_to_kana),
            InputStyle::Mapped(InputTableId::DefaultAzik) => Some(&self.azik),
            InputStyle::Mapped(InputTableId::DefaultKanaJis) => Some(&self.kana_jis),
            InputStyle::Mapped(InputTableId::DefaultKanaUs) => Some(&self.kana_us),
            InputStyle::Mapped(InputTableId::Named(name)) => {
                Some(self.custom.get(name).unwrap_or(&self.empty))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InputElement {
    pub piece: InputPiece,
    pub input_style: InputStyle,
}

impl InputElement {
    pub fn new(piece: InputPiece, input_style: InputStyle) -> Self {
        Self { piece, input_style }
    }

    pub fn character(value: impl Into<String>, input_style: InputStyle) -> Self {
        Self::new(InputPiece::Character(value.into()), input_style)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComposingCount {
    Input(usize),
    Surface(usize),
    Composite(Box<ComposingCount>, Box<ComposingCount>),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComposingText {
    cursor: usize,
    input: Vec<InputElement>,
    surface: Vec<Grapheme>,
}

#[derive(Clone, Copy, Debug)]
struct IndexPair {
    input: usize,
    surface: usize,
}

#[derive(Clone, Debug)]
struct SurfaceSegment {
    graphemes: Vec<Grapheme>,
    input_style: InputStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DifferenceSuffix {
    pub deleted_input: usize,
    pub added_input: usize,
    pub deleted_surface: usize,
    pub added_surface: usize,
}

impl ComposingText {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn input(&self) -> &[InputElement] {
        &self.input
    }

    pub fn surface(&self) -> String {
        self.surface.concat()
    }

    pub fn surface_graphemes(&self) -> &[Grapheme] {
        &self.surface
    }

    pub fn surface_before_cursor(&self) -> String {
        self.surface[..self.cursor].concat()
    }

    pub fn is_empty(&self) -> bool {
        self.surface.is_empty()
    }

    pub fn is_at_end(&self) -> bool {
        self.cursor == self.surface.len()
    }

    pub fn is_at_start(&self) -> bool {
        self.cursor == 0
    }

    pub fn insert_str(
        &mut self,
        value: &str,
        input_style: InputStyle,
        tables: &InputTableRegistry,
    ) {
        self.insert(
            graphemes(value)
                .into_iter()
                .map(|value| InputElement::character(value, input_style.clone()))
                .collect(),
            tables,
        );
    }

    pub fn insert(&mut self, mut elements: Vec<InputElement>, tables: &InputTableRegistry) {
        if elements.is_empty() {
            return;
        }

        let mut input_cursor = self.force_input_cursor(self.cursor, tables);
        if !self.is_at_end() {
            elements.push(InputElement::new(
                InputPiece::CompositionSeparator,
                InputStyle::frozen(),
            ));
        }

        if input_cursor > 0
            && self.input[input_cursor - 1].piece == InputPiece::CompositionSeparator
            && self.input[input_cursor - 1].input_style == InputStyle::frozen()
        {
            self.input.remove(input_cursor - 1);
            input_cursor -= 1;
        }

        let inserted_count = elements.len();
        self.input.splice(input_cursor..input_cursor, elements);

        let old_prefix = self.surface[..self.cursor].to_vec();
        let new_prefix =
            Self::convert_elements(&self.input[..input_cursor + inserted_count], tables);
        let common = common_prefix_len(&old_prefix, &new_prefix);
        let deleted = old_prefix.len() - common;
        let added = new_prefix.len() - common;
        self.surface.splice(0..self.cursor, new_prefix);
        self.cursor = self.cursor - deleted + added;
    }

    pub fn delete_forward(&mut self, count: usize, tables: &InputTableRegistry) {
        let count = count.min(self.surface.len() - self.cursor);
        if count == 0 {
            return;
        }
        self.cursor += count;
        self.delete_backward(count, tables);
    }

    pub fn delete_backward(&mut self, count: usize, tables: &InputTableRegistry) {
        let count = count.min(self.cursor);
        if count == 0 {
            return;
        }

        let mut target = self.force_input_cursor(self.cursor - count, tables);
        let mut current = self.force_input_cursor(self.cursor, tables);
        if target > 0
            && self.input[target - 1].piece == InputPiece::CompositionSeparator
            && self.input[target - 1].input_style == InputStyle::frozen()
        {
            self.input.remove(target - 1);
            target -= 1;
            current -= 1;
        }

        if target == 0 || current == self.input.len() {
            self.input.drain(target..current);
        } else {
            self.input.splice(
                target..current,
                [InputElement::new(
                    InputPiece::CompositionSeparator,
                    InputStyle::frozen(),
                )],
            );
        }
        self.cursor -= count;
        self.surface = Self::convert_elements(&self.input, tables);
    }

    pub fn move_cursor(&mut self, count: isize) -> isize {
        let minimum = -(self.cursor as isize);
        let maximum = (self.surface.len() - self.cursor) as isize;
        let moved = count.clamp(minimum, maximum);
        self.cursor = self.cursor.saturating_add_signed(moved);
        moved
    }

    pub fn complete_prefix(&mut self, count: ComposingCount, tables: &InputTableRegistry) {
        match count {
            ComposingCount::Input(count) => {
                let count = count.min(self.input.len());
                self.input.drain(..count);
                let new_surface = Self::convert_elements(&self.input, tables);
                let cursor_delta = self.surface.len() - new_surface.len();
                self.surface = new_surface;
                self.cursor = self.cursor.saturating_sub(cursor_delta);
                if self.cursor == 0 {
                    self.cursor = self.surface.len();
                }
            }
            ComposingCount::Surface(count) => {
                let count = count.min(self.surface.len());
                let input_index = self.force_input_cursor(count, tables);
                self.input.drain(..input_index);
                self.surface.drain(..count);
                self.cursor = self.cursor.saturating_sub(count);
                if self.cursor == 0 {
                    self.cursor = self.surface.len();
                }
            }
            ComposingCount::Composite(left, right) => {
                self.complete_prefix(*left, tables);
                self.complete_prefix(*right, tables);
            }
        }
    }

    pub fn prefix_to_cursor(&self, tables: &InputTableRegistry) -> Self {
        let mut text = self.clone();
        let input_index = text.force_input_cursor(text.cursor, tables);
        text.input.truncate(input_index);
        text.surface.truncate(text.cursor);
        text
    }

    pub fn input_to_surface_map(&self, tables: &InputTableRegistry) -> HashMap<usize, usize> {
        if self.has_identity_mapping() {
            return (0..=self.input.len()).map(|index| (index, index)).collect();
        }
        self.independent_boundaries(tables)
            .into_iter()
            .map(|pair| (pair.input, pair.surface))
            .collect()
    }

    pub fn difference_suffix(&self, previous: &Self) -> DifferenceSuffix {
        let input_common = common_prefix_len(&self.input, &previous.input);
        let surface_common = common_prefix_len(&self.surface, &previous.surface);
        DifferenceSuffix {
            deleted_input: previous.input.len() - input_common,
            added_input: self.input.len() - input_common,
            deleted_surface: previous.surface.len() - surface_common,
            added_surface: self.surface.len() - surface_common,
        }
    }

    pub fn input_has_suffix(&self, suffix: &Self) -> bool {
        self.input.ends_with(&suffix.input)
    }

    pub fn stop(&mut self) {
        self.input.clear();
        self.surface.clear();
        self.cursor = 0;
    }

    fn has_identity_mapping(&self) -> bool {
        self.input.len() == self.surface.len()
            && self.input.iter().all(|element| {
                element.input_style == InputStyle::Direct
                    && matches!(element.piece, InputPiece::Character(_))
            })
    }

    fn independent_boundaries(&self, tables: &InputTableRegistry) -> Vec<IndexPair> {
        let mut boundaries = vec![IndexPair {
            input: 0,
            surface: 0,
        }];
        let mut converting: Vec<SurfaceSegment> = Vec::new();
        let mut converted_len = 0;

        for (input_index, element) in self.input.iter().enumerate() {
            let previous_segment_count = converting.len();
            let previous_tail_len = converting
                .last()
                .map(|segment| segment.graphemes.len())
                .unwrap_or(0);
            let deleted = update_segments(&mut converting, element, tables);
            let previous_converted_len = converted_len;
            let current_tail_len = converting
                .last()
                .map(|segment| segment.graphemes.len())
                .unwrap_or(0);
            if converting.len() == previous_segment_count {
                converted_len = converted_len - previous_tail_len + current_tail_len;
            } else {
                converted_len += current_tail_len;
            }

            while let Some(last) = boundaries.pop() {
                if last.surface <= previous_converted_len.saturating_sub(deleted) {
                    boundaries.push(last);
                    break;
                }
            }
            boundaries.push(IndexPair {
                input: input_index + 1,
                surface: converted_len,
            });
        }
        boundaries
    }

    fn force_input_cursor(&mut self, target_surface: usize, tables: &InputTableRegistry) -> usize {
        if target_surface == 0 {
            return 0;
        }
        if target_surface >= self.surface.len() {
            return self.input.len();
        }

        let mut boundaries = self.independent_boundaries(tables);
        let mut segment_end = IndexPair {
            input: self.input.len(),
            surface: self.surface.len(),
        };
        let mut segment_start = IndexPair {
            input: 0,
            surface: 0,
        };
        while let Some(start) = boundaries.pop() {
            if start.surface == target_surface {
                return start.input;
            }
            if start.surface < target_surface {
                segment_start = start;
                break;
            }
            if start.surface < segment_end.surface {
                segment_end = start;
            }
        }

        let frozen = self.surface[segment_start.surface..segment_end.surface]
            .iter()
            .cloned()
            .map(|value| InputElement::character(value, InputStyle::frozen()));
        self.input
            .splice(segment_start.input..segment_end.input, frozen);
        target_surface - segment_start.surface + segment_start.input
    }

    fn convert_elements(elements: &[InputElement], tables: &InputTableRegistry) -> Vec<Grapheme> {
        let mut segments = Vec::new();
        for element in elements {
            update_segments(&mut segments, element, tables);
        }
        segments
            .into_iter()
            .flat_map(|segment| segment.graphemes)
            .collect()
    }
}

fn update_segments(
    segments: &mut Vec<SurfaceSegment>,
    element: &InputElement,
    tables: &InputTableRegistry,
) -> usize {
    if let Some(last) = segments.last_mut()
        && last.input_style == element.input_style
    {
        return update_surface(
            &mut last.graphemes,
            &element.piece,
            &element.input_style,
            tables,
        );
    }
    if segments.is_empty() && element.piece == InputPiece::CompositionSeparator {
        return 0;
    }
    let mut surface = Vec::new();
    update_surface(&mut surface, &element.piece, &element.input_style, tables);
    segments.push(SurfaceSegment {
        graphemes: surface,
        input_style: element.input_style.clone(),
    });
    0
}

fn update_surface(
    surface: &mut Vec<Grapheme>,
    piece: &InputPiece,
    style: &InputStyle,
    tables: &InputTableRegistry,
) -> usize {
    if let Some(table) = tables.resolve(style) {
        table.apply(surface, piece.clone())
    } else {
        if let Some(value) = piece.displayed() {
            surface.push(value.to_owned());
        }
        0
    }
}

fn common_prefix_len<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_direct_input_by_grapheme() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str("あいうえお", InputStyle::Direct, &tables);
        assert_eq!(text.surface(), "あいうえお");
        assert_eq!(text.cursor(), 5);

        assert_eq!(text.move_cursor(-3), -3);
        text.delete_forward(1, &tables);
        text.insert_str("宇", InputStyle::Direct, &tables);

        assert_eq!(text.surface(), "あい宇えお");
        assert_eq!(text.cursor(), 3);
    }

    #[test]
    fn preserves_roman_history_separately_from_surface() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();

        text.insert_str("kyouhaame", InputStyle::RomanToKana, &tables);

        assert_eq!(text.surface(), "きょうはあめ");
        assert_eq!(text.input().len(), 9);
        assert_eq!(text.cursor(), 6);
    }

    #[test]
    fn freezes_a_roman_segment_before_middle_insertion() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str("nana", InputStyle::RomanToKana, &tables);
        text.move_cursor(-1);

        text.insert_str("k", InputStyle::RomanToKana, &tables);
        assert_eq!(text.surface(), "なkな");
        text.insert_str("a", InputStyle::RomanToKana, &tables);

        assert_eq!(text.surface(), "なかな");
        assert_eq!(text.cursor(), 2);
    }

    #[test]
    fn deletion_does_not_recombine_across_the_removed_region() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str("atst", InputStyle::RomanToKana, &tables);
        text.move_cursor(-2);

        text.delete_forward(1, &tables);
        text.move_cursor(1);

        assert_eq!(text.surface(), "あtt");
    }

    #[test]
    fn resolves_n_at_a_composition_boundary() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str("an", InputStyle::RomanToKana, &tables);
        assert_eq!(text.surface(), "あn");

        text.insert(
            vec![InputElement::new(
                InputPiece::CompositionSeparator,
                InputStyle::RomanToKana,
            )],
            &tables,
        );

        assert_eq!(text.surface(), "あん");
    }

    #[test]
    fn maps_only_independent_roman_boundaries() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str("kyouhaiitenkida", InputStyle::RomanToKana, &tables);

        let map = text.input_to_surface_map(&tables);
        assert_eq!(map.get(&0), Some(&0));
        assert_eq!(map.get(&3), Some(&2));
        assert_eq!(map.get(&4), Some(&3));
        assert_eq!(map.get(&6), Some(&4));
        assert_eq!(map.get(&13), Some(&9));
        assert_eq!(map.get(&15), Some(&10));
        assert!(!map.contains_key(&1));
    }

    #[test]
    fn completes_a_surface_prefix_without_splitting_a_roman_segment() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str("danbasu", InputStyle::RomanToKana, &tables);

        text.complete_prefix(ComposingCount::Surface(2), &tables);

        assert_eq!(text.surface(), "ばす");
        assert_eq!(text.input()[0].piece, InputPiece::character("ば"));
        assert_eq!(text.input()[0].input_style, InputStyle::frozen());
    }

    #[test]
    fn composes_and_partially_completes_azik_input() {
        let tables = InputTableRegistry::new();
        let mut text = ComposingText::new();
        text.insert_str(
            "szzz",
            InputStyle::Mapped(InputTableId::DefaultAzik),
            &tables,
        );
        assert_eq!(text.surface(), "さんざん");

        text.complete_prefix(ComposingCount::Surface(3), &tables);

        assert_eq!(text.surface(), "ん");
        assert_eq!(text.input()[0].piece, InputPiece::character("ん"));
    }
}
