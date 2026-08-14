#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct DictionaryMetadata(u32);

impl DictionaryMetadata {
    pub const LEARNED: Self = Self(1 << 0);
    pub const USER_DICTIONARY: Self = Self(1 << 1);

    pub fn contains(self, value: Self) -> bool {
        self.0 & value.0 == value.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DictionaryEntry {
    pub word: String,
    pub ruby: String,
    pub left_id: u16,
    pub right_id: u16,
    pub meaning_id: u16,
    pub base_value: f32,
    pub adjustment: f32,
    pub metadata: DictionaryMetadata,
}

impl DictionaryEntry {
    pub fn value(&self) -> f32 {
        0.0_f32.min(self.base_value + self.adjustment)
    }

    pub fn adjusted(mut self, value: f32) -> Self {
        self.adjustment += value;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_value_is_never_positive() {
        let mut entry = DictionaryEntry {
            word: "例".into(),
            ruby: "レイ".into(),
            left_id: 1,
            right_id: 2,
            meaning_id: 3,
            base_value: -2.0,
            adjustment: 1.5,
            metadata: DictionaryMetadata::default(),
        };
        assert_eq!(entry.value(), -0.5);

        entry.adjustment = 5.0;
        assert_eq!(entry.value(), 0.0);
    }
}
