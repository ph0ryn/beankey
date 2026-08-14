use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    ComposingText, InputElement, InputPiece, InputStyle, InputTableId, InputTableRegistry,
};

const MAXIMUM_PENALTY: f32 = 10.5;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TypoPrefix {
    pub ruby: String,
    pub consumed: usize,
    pub penalty: f32,
}

pub(crate) fn typo_prefixes(
    input: &[InputElement],
    tables: &InputTableRegistry,
    maximum_length: usize,
) -> Vec<TypoPrefix> {
    let mut prefixes = HashMap::new();
    enumerate(
        input,
        tables,
        maximum_length,
        0,
        0.0,
        false,
        &mut Vec::new(),
        &mut prefixes,
    );
    prefixes
        .into_iter()
        .map(|((ruby, consumed), penalty)| TypoPrefix {
            ruby,
            consumed,
            penalty,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn enumerate(
    input: &[InputElement],
    tables: &InputTableRegistry,
    maximum_length: usize,
    consumed: usize,
    penalty: f32,
    changed: bool,
    elements: &mut Vec<InputElement>,
    output: &mut HashMap<(String, usize), f32>,
) {
    if consumed > 0 && changed {
        let surface = render(elements, tables);
        let ruby = to_katakana(&surface);
        let ruby_count = UnicodeSegmentation::graphemes(ruby.as_str(), true).count();
        if ruby_count <= maximum_length && is_stable_prefix(input, consumed, elements, tables) {
            output
                .entry((ruby, consumed))
                .and_modify(|current| *current = current.min(penalty))
                .or_insert(penalty);
        }
        if ruby_count >= maximum_length {
            return;
        }
    }
    if consumed >= input.len() || consumed >= maximum_length {
        return;
    }

    elements.push(input[consumed].clone());
    enumerate(
        input,
        tables,
        maximum_length,
        consumed + 1,
        penalty,
        changed,
        elements,
        output,
    );
    elements.pop();

    if penalty >= MAXIMUM_PENALTY {
        return;
    }
    for &(replacement, weight) in direct_replacements(&input[consumed]) {
        elements.push(InputElement::character(replacement, InputStyle::Direct));
        enumerate(
            input,
            tables,
            maximum_length,
            consumed + 1,
            penalty + weight,
            true,
            elements,
            output,
        );
        elements.pop();
    }
    if consumed + 1 < input.len()
        && let Some(replacement) = roman_replacement(&input[consumed..consumed + 2])
    {
        let previous_length = elements.len();
        elements.extend(replacement.chars().map(|character| {
            InputElement::character(character.to_string(), InputStyle::RomanToKana)
        }));
        enumerate(
            input,
            tables,
            maximum_length,
            consumed + 2,
            penalty + 3.5,
            true,
            elements,
            output,
        );
        elements.truncate(previous_length);
    }
}

fn render(elements: &[InputElement], tables: &InputTableRegistry) -> String {
    let mut composing = ComposingText::new();
    composing.insert(elements.to_vec(), tables);
    composing.surface()
}

fn is_stable_prefix(
    input: &[InputElement],
    consumed: usize,
    elements: &[InputElement],
    tables: &InputTableRegistry,
) -> bool {
    if consumed >= input.len() {
        return true;
    }
    let surface = render(elements, tables);
    let mut extended = elements.to_vec();
    extended.push(input[consumed].clone());
    render(&extended, tables).starts_with(&surface)
}

fn direct_replacements(element: &InputElement) -> &'static [(&'static str, f32)] {
    if element.input_style != InputStyle::Direct {
        return &[];
    }
    let InputPiece::Character(value) = &element.piece else {
        return &[];
    };
    match to_katakana(value).as_str() {
        "カ" => &[("ガ", 7.0)],
        "キ" => &[("ギ", 3.5)],
        "ク" => &[("グ", 3.5)],
        "ケ" => &[("ゲ", 3.5)],
        "コ" => &[("ゴ", 3.5)],
        "サ" => &[("ザ", 3.5)],
        "シ" => &[("ジ", 3.5)],
        "ス" => &[("ズ", 3.5)],
        "セ" => &[("ゼ", 3.5)],
        "ソ" => &[("ゾ", 3.5)],
        "タ" => &[("ダ", 6.0)],
        "チ" => &[("ヂ", 3.5)],
        "ツ" => &[("ッ", 6.0), ("ヅ", 4.5)],
        "テ" => &[("デ", 6.0)],
        "ト" => &[("ド", 4.5)],
        "ハ" => &[("バ", 4.5), ("パ", 6.0)],
        "ヒ" => &[("ビ", 3.5), ("ピ", 4.5)],
        "フ" => &[("ブ", 3.5), ("プ", 4.5)],
        "ヘ" => &[("ベ", 3.5), ("ペ", 4.5)],
        "ホ" => &[("ボ", 3.5), ("ポ", 4.5)],
        "バ" => &[("パ", 3.5)],
        "ビ" => &[("ピ", 3.5)],
        "ブ" => &[("プ", 3.5)],
        "ベ" => &[("ペ", 3.5)],
        "ボ" => &[("ポ", 3.5)],
        "ヤ" => &[("ャ", 3.5)],
        "ユ" => &[("ュ", 3.5)],
        "ヨ" => &[("ョ", 3.5)],
        _ => &[],
    }
}

fn roman_replacement(elements: &[InputElement]) -> Option<&'static str> {
    if !elements.iter().all(|element| {
        matches!(
            element.input_style,
            InputStyle::RomanToKana | InputStyle::Mapped(InputTableId::DefaultRomanToKana)
        )
    }) {
        return None;
    }
    let mut key = String::new();
    for element in elements {
        let InputPiece::Character(value) = &element.piece else {
            return None;
        };
        key.push_str(value);
    }
    match key.as_str() {
        "bs" => Some("ba"),
        "no" => Some("bo"),
        "li" => Some("ki"),
        "lo" => Some("ko"),
        "lu" => Some("ku"),
        "my" => Some("mu"),
        "tp" => Some("to"),
        "ts" => Some("ta"),
        "wi" => Some("wo"),
        "pu" => Some("ou"),
        _ => None,
    }
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

    #[test]
    fn enumerates_direct_and_roman_rules_from_the_fixed_upstream() {
        let tables = InputTableRegistry::new();
        let direct = "たいかくせい"
            .chars()
            .map(|character| InputElement::character(character.to_string(), InputStyle::Direct))
            .collect::<Vec<_>>();
        assert!(
            typo_prefixes(&direct, &tables, 20)
                .iter()
                .any(|prefix| prefix.ruby == "タイガクセイ" && prefix.penalty == 7.0)
        );

        let roman = "li"
            .chars()
            .map(|character| {
                InputElement::character(character.to_string(), InputStyle::RomanToKana)
            })
            .collect::<Vec<_>>();
        assert!(
            typo_prefixes(&roman, &tables, 20)
                .iter()
                .any(|prefix| prefix.ruby == "キ" && prefix.penalty == 3.5)
        );
    }
}
