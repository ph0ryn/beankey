use unicode_segmentation::UnicodeSegmentation;

const INPUT_TAG: char = '\u{EE00}';
const OUTPUT_TAG: char = '\u{EE01}';
const CONTEXT_TAG: char = '\u{EE02}';
const PROFILE_TAG: char = '\u{EE03}';
const TOPIC_TAG: char = '\u{EE04}';
const STYLE_TAG: char = '\u{EE05}';
const PREFERENCE_TAG: char = '\u{EE06}';
const RIGHT_CONTEXT_TAG: char = '\u{EE07}';
pub const ALIGNMENT_SEPARATOR: char = '\u{EE08}';

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct PrefixConstraint {
    pub bytes: Vec<u8>,
    pub has_eos: bool,
    pub ignore_memory_and_user_dictionary: bool,
}

impl PrefixConstraint {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            ..Self::default()
        }
    }

    pub fn normalized(
        bytes: Vec<u8>,
        default_has_eos: bool,
        ignore_memory_and_user_dictionary: bool,
    ) -> Self {
        let separator = ALIGNMENT_SEPARATOR.to_string().into_bytes();
        let alignment = bytes
            .windows(separator.len())
            .position(|window| window == separator);
        match alignment {
            Some(index) => Self {
                bytes: bytes[..index].to_vec(),
                has_eos: true,
                ignore_memory_and_user_dictionary,
            },
            None => Self {
                bytes,
                has_eos: default_has_eos,
                ignore_memory_and_user_dictionary,
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty() && !self.has_eos
    }

    pub(crate) fn can_continue(&self, candidate: &[u8]) -> bool {
        if self.has_eos {
            candidate.len() <= self.bytes.len() && self.bytes.starts_with(candidate)
        } else {
            self.bytes.starts_with(candidate) || candidate.starts_with(&self.bytes)
        }
    }

    pub(crate) fn is_satisfied_by(&self, candidate: &[u8]) -> bool {
        if self.has_eos {
            candidate == self.bytes
        } else {
            candidate.starts_with(&self.bytes)
        }
    }
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct ZenzV2Config {
    pub profile: Option<String>,
    pub left_context: Option<String>,
    pub max_left_context_length: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct ZenzV3Config {
    pub profile: Option<String>,
    pub topic: Option<String>,
    pub style: Option<String>,
    pub preference: Option<String>,
    pub left_context: Option<String>,
    pub right_context: Option<String>,
    pub max_left_context_length: Option<usize>,
    pub max_right_context_length: Option<usize>,
    pub enable_alignment_separator: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ZenzVersionConfig {
    V2(ZenzV2Config),
    V3(ZenzV3Config),
}

impl Default for ZenzVersionConfig {
    fn default() -> Self {
        Self::V3(ZenzV3Config::default())
    }
}

pub struct ZenzPromptBuilder;

impl ZenzPromptBuilder {
    pub fn input_prediction(
        left_context: &str,
        composing_text: &str,
        version: &ZenzVersionConfig,
    ) -> Option<String> {
        let ZenzVersionConfig::V3(config) = version else {
            return None;
        };
        let mut prompt = v3_conditions(config).concat();
        let left_context = suffix(left_context, config.max_left_context_length.unwrap_or(40));
        let right_context = prefix(
            config.right_context.as_deref().unwrap_or_default(),
            config.max_right_context_length.unwrap_or(40),
        );
        if !left_context.is_empty() {
            prompt.push(CONTEXT_TAG);
            prompt.push_str(&left_context);
        }
        if !right_context.is_empty() {
            prompt.push(RIGHT_CONTEXT_TAG);
            prompt.push_str(&right_context);
        }
        prompt.push(INPUT_TAG);
        prompt.push_str(&to_katakana(composing_text));
        Some(prompt)
    }

    pub fn typo_correction_prefix(left_context: &str) -> String {
        if left_context.is_empty() {
            INPUT_TAG.to_string()
        } else {
            format!("{CONTEXT_TAG}{left_context}{INPUT_TAG}")
        }
    }

    pub fn candidate_evaluation(
        input: &str,
        input_cursor_position: Option<usize>,
        user_dictionary_prompt: &str,
        version: &ZenzVersionConfig,
    ) -> String {
        let mut conditions = Vec::new();
        if !user_dictionary_prompt.is_empty() {
            conditions.push(format!("辞書:{user_dictionary_prompt}"));
        }

        match version {
            ZenzVersionConfig::V2(config) => {
                if let Some(profile) = config.profile.as_deref().filter(|value| !value.is_empty()) {
                    conditions.push(format!("プロフィール:{}", suffix(profile, 25)));
                }
                let left_context = suffix(
                    config.left_context.as_deref().unwrap_or_default(),
                    config.max_left_context_length.unwrap_or(40),
                );
                if !conditions.is_empty() {
                    format!(
                        "{INPUT_TAG}{input}{CONTEXT_TAG}{}・発言:{left_context}{OUTPUT_TAG}",
                        conditions.join("・")
                    )
                } else if !left_context.is_empty() {
                    format!("{INPUT_TAG}{input}{CONTEXT_TAG}{left_context}{OUTPUT_TAG}")
                } else {
                    format!("{INPUT_TAG}{input}{OUTPUT_TAG}")
                }
            }
            ZenzVersionConfig::V3(config) => {
                let mut prompt = v3_conditions(config).concat();
                if !user_dictionary_prompt.is_empty() {
                    prompt.insert_str(0, &format!("辞書:{user_dictionary_prompt}"));
                }
                let left_context = suffix(
                    config.left_context.as_deref().unwrap_or_default(),
                    config.max_left_context_length.unwrap_or(40),
                );
                let right_context = prefix(
                    config.right_context.as_deref().unwrap_or_default(),
                    config.max_right_context_length.unwrap_or(40),
                );
                if !left_context.is_empty() {
                    prompt.push(CONTEXT_TAG);
                    prompt.push_str(&left_context);
                }
                if !right_context.is_empty() {
                    prompt.push(RIGHT_CONTEXT_TAG);
                    prompt.push_str(&right_context);
                }
                prompt.push(INPUT_TAG);
                prompt.push_str(&aligned_input(
                    input,
                    input_cursor_position,
                    config.enable_alignment_separator,
                ));
                prompt.push(OUTPUT_TAG);
                prompt
            }
        }
    }

    pub fn candidate_text_for_evaluation(
        candidate_text: &str,
        input: &str,
        input_cursor_position: Option<usize>,
        version: &ZenzVersionConfig,
    ) -> String {
        let append_separator = matches!(
            version,
            ZenzVersionConfig::V3(ZenzV3Config {
                enable_alignment_separator: true,
                ..
            })
        ) && should_insert_alignment_separator(input, input_cursor_position);
        if append_separator {
            format!("{candidate_text}{ALIGNMENT_SEPARATOR}")
        } else {
            candidate_text.to_owned()
        }
    }
}

fn v3_conditions(config: &ZenzV3Config) -> Vec<String> {
    [
        (PROFILE_TAG, config.profile.as_deref()),
        (TOPIC_TAG, config.topic.as_deref()),
        (STYLE_TAG, config.style.as_deref()),
        (PREFERENCE_TAG, config.preference.as_deref()),
    ]
    .into_iter()
    .filter_map(|(tag, value)| {
        value
            .filter(|value| !value.is_empty())
            .map(|value| format!("{tag}{}", suffix(value, 25)))
    })
    .collect()
}

fn aligned_input(input: &str, cursor_position: Option<usize>, enabled: bool) -> String {
    if !enabled || !should_insert_alignment_separator(input, cursor_position) {
        return input.to_owned();
    }
    let cursor_position = cursor_position.expect("validated cursor position");
    let mut graphemes = UnicodeSegmentation::graphemes(input, true);
    let left = graphemes.by_ref().take(cursor_position).collect::<String>();
    format!(
        "{left}{ALIGNMENT_SEPARATOR}{}",
        graphemes.collect::<String>()
    )
}

fn should_insert_alignment_separator(input: &str, cursor_position: Option<usize>) -> bool {
    cursor_position
        .is_some_and(|position| position < UnicodeSegmentation::graphemes(input, true).count())
}

fn prefix(value: &str, count: usize) -> String {
    UnicodeSegmentation::graphemes(value, true)
        .take(count)
        .collect()
}

fn suffix(value: &str, count: usize) -> String {
    let graphemes = UnicodeSegmentation::graphemes(value, true).collect::<Vec<_>>();
    graphemes[graphemes.len().saturating_sub(count)..].concat()
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
    fn builds_v3_input_prediction_prompt() {
        let config = ZenzVersionConfig::V3(ZenzV3Config {
            profile: Some("profile".into()),
            topic: Some("topic".into()),
            style: Some("style".into()),
            preference: Some("preference".into()),
            right_context: Some("uvwxyz".into()),
            max_left_context_length: Some(2),
            max_right_context_length: Some(3),
            ..Default::default()
        });
        assert_eq!(
            ZenzPromptBuilder::input_prediction("abcdef", "かんじ", &config).unwrap(),
            "\u{EE03}profile\u{EE04}topic\u{EE05}style\u{EE06}preference\u{EE02}ef\u{EE07}uvw\u{EE00}カンジ"
        );
    }

    #[test]
    fn builds_v2_candidate_evaluation_prompt() {
        let config = ZenzVersionConfig::V2(ZenzV2Config {
            profile: Some("profile".into()),
            left_context: Some("abcdef".into()),
            max_left_context_length: Some(3),
        });
        assert_eq!(
            ZenzPromptBuilder::candidate_evaluation("ヘンカン", None, "単語(たんご)", &config),
            "\u{EE00}ヘンカン\u{EE02}辞書:単語(たんご)・プロフィール:profile・発言:def\u{EE01}"
        );
    }

    #[test]
    fn inserts_alignment_separator_only_for_an_internal_cursor() {
        let config = ZenzVersionConfig::V3(ZenzV3Config {
            enable_alignment_separator: true,
            ..Default::default()
        });
        assert_eq!(
            ZenzPromptBuilder::candidate_evaluation("ハシ", Some(1), "", &config),
            "\u{EE00}ハ\u{EE08}シ\u{EE01}"
        );
        assert_eq!(
            ZenzPromptBuilder::candidate_text_for_evaluation("葉", "ハシ", Some(1), &config),
            "葉\u{EE08}"
        );
        assert_eq!(
            ZenzPromptBuilder::candidate_evaluation("ハシ", Some(2), "", &config),
            "\u{EE00}ハシ\u{EE01}"
        );
    }

    #[test]
    fn builds_context_only_prompts() {
        assert_eq!(ZenzPromptBuilder::typo_correction_prefix(""), "\u{EE00}");
        assert_eq!(
            ZenzPromptBuilder::candidate_evaluation(
                "ハシ",
                None,
                "",
                &ZenzVersionConfig::V3(ZenzV3Config {
                    right_context: Some("abcdef".into()),
                    max_right_context_length: Some(2),
                    ..Default::default()
                })
            ),
            "\u{EE07}ab\u{EE00}ハシ\u{EE01}"
        );
    }

    #[test]
    fn alignment_separator_turns_a_prefix_into_an_eos_constraint() {
        let constraint =
            PrefixConstraint::normalized("橋\u{EE08}ignored".as_bytes().to_vec(), false, true);
        assert_eq!(constraint.bytes, "橋".as_bytes());
        assert!(constraint.has_eos);
        assert!(constraint.ignore_memory_and_user_dictionary);
    }
}
