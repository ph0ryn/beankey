pub fn to_katakana(value: &str) -> String {
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

pub fn to_hiragana(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{30a1}'..='\u{30f6}' => {
                char::from_u32(u32::from(character) - 96).expect("hiragana scalar is valid")
            }
            _ => character,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_only_the_fixed_kana_ranges() {
        assert_eq!(to_katakana("ぁあゖー漢"), "ァアヶー漢");
        assert_eq!(to_hiragana("ァアヶー漢"), "ぁあゖー漢");
    }
}
