use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt;

use super::table::graphemes;
use super::{InputModifier, InputPiece, InputTable, KeyElement, ValueElement};

const COMPOSITION_SEPARATOR: &str = "composition-separator";
const ANY_CHARACTER: &str = "any character";
const LEFT_BRACKET: &str = "lbracket";
const RIGHT_BRACKET: &str = "rbracket";
const SHIFT_ZERO: &str = "shift 0";
const SHIFT_UNDERSCORE: &str = "shift _";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FormatSide {
    Key,
    Value,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum FormatErrorKind {
    InvalidTabCount { found: usize },
    UnknownBraceToken { token: String, side: FormatSide },
    UnclosedBrace,
    ShiftTokenNotAtTail { token: String },
    DuplicateRule { first_defined_at: usize },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FormatError {
    pub line: usize,
    pub kind: FormatErrorKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatReport {
    FullyValid,
    InvalidLines(Vec<FormatError>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputTableExportError {
    element: KeyElement,
}

impl fmt::Display for InputTableExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported input-table key element: {:?}",
            self.element
        )
    }
}

impl Error for InputTableExportError {}

impl InputTable {
    pub fn from_custom_tsv(content: &str) -> Self {
        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let columns: Vec<_> = line
                .split('\t')
                .filter(|column| !column.is_empty())
                .collect();
            if columns.len() < 2 {
                continue;
            }
            entries.push((parse_key(columns[0]), parse_value(columns[1])));
        }
        Self::new(entries)
    }

    pub fn to_custom_tsv(&self) -> Result<String, InputTableExportError> {
        self.entries()
            .map(|(key, value)| {
                let key = key
                    .iter()
                    .map(encode_key)
                    .collect::<Result<String, InputTableExportError>>()?;
                let value = value.iter().map(encode_value).collect::<String>();
                Ok(format!("{key}\t{value}"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|lines| lines.join("\n"))
    }

    pub fn check_custom_tsv(content: &str) -> FormatReport {
        let mut errors = Vec::new();
        let mut first_seen: HashMap<Vec<KeyElement>, usize> = HashMap::new();
        for (line_number, line) in content.split('\n').enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let tab_count = line.bytes().filter(|byte| *byte == b'\t').count();
            if tab_count != 1 {
                errors.push(FormatError {
                    line: line_number,
                    kind: FormatErrorKind::InvalidTabCount { found: tab_count },
                });
            }
            let Some((key, value)) = line.split_once('\t') else {
                continue;
            };

            let shift = scan_tokens(key, FormatSide::Key, line_number, &mut errors);
            scan_tokens(value, FormatSide::Value, line_number, &mut errors);
            if let Some(token) = shift.not_at_tail {
                errors.push(FormatError {
                    line: line_number,
                    kind: FormatErrorKind::ShiftTokenNotAtTail { token },
                });
            }

            let parsed = parse_key(key);
            if let Some(first_defined_at) = first_seen.get(&parsed) {
                errors.push(FormatError {
                    line: line_number,
                    kind: FormatErrorKind::DuplicateRule {
                        first_defined_at: *first_defined_at,
                    },
                });
            } else {
                first_seen.insert(parsed, line_number);
            }
        }

        if errors.is_empty() {
            FormatReport::FullyValid
        } else {
            FormatReport::InvalidLines(errors)
        }
    }
}

#[derive(Default)]
struct ShiftScan {
    not_at_tail: Option<String>,
}

fn scan_tokens(
    value: &str,
    side: FormatSide,
    line: usize,
    errors: &mut Vec<FormatError>,
) -> ShiftScan {
    let known_key = HashSet::from([
        COMPOSITION_SEPARATOR,
        ANY_CHARACTER,
        LEFT_BRACKET,
        RIGHT_BRACKET,
        SHIFT_ZERO,
        SHIFT_UNDERSCORE,
    ]);
    let known_value = HashSet::from([ANY_CHARACTER, LEFT_BRACKET, RIGHT_BRACKET]);
    let known = match side {
        FormatSide::Key => &known_key,
        FormatSide::Value => &known_value,
    };
    let mut scan = ShiftScan::default();
    let mut remaining = value;
    while let Some(open) = remaining.find('{') {
        remaining = &remaining[open + 1..];
        let Some(close) = remaining.find('}') else {
            errors.push(FormatError {
                line,
                kind: FormatErrorKind::UnclosedBrace,
            });
            break;
        };
        let token = &remaining[..close];
        if token.contains('{') {
            errors.push(FormatError {
                line,
                kind: FormatErrorKind::UnclosedBrace,
            });
        } else {
            if !known.contains(token) {
                errors.push(FormatError {
                    line,
                    kind: FormatErrorKind::UnknownBraceToken {
                        token: token.to_owned(),
                        side,
                    },
                });
            }
            if side == FormatSide::Key
                && matches!(token, SHIFT_ZERO | SHIFT_UNDERSCORE)
                && close + 1 != remaining.len()
                && scan.not_at_tail.is_none()
            {
                scan.not_at_tail = Some(token.to_owned());
            }
        }
        remaining = &remaining[close + 1..];
    }
    scan
}

fn parse_key(value: &str) -> Vec<KeyElement> {
    parse_elements(
        value,
        |token| match token {
            COMPOSITION_SEPARATOR => Some(KeyElement::Piece(InputPiece::CompositionSeparator)),
            ANY_CHARACTER => Some(KeyElement::Any),
            LEFT_BRACKET => Some(KeyElement::Piece(InputPiece::character("{"))),
            RIGHT_BRACKET => Some(KeyElement::Piece(InputPiece::character("}"))),
            SHIFT_ZERO => Some(KeyElement::Piece(shifted_key("0"))),
            SHIFT_UNDERSCORE => Some(KeyElement::Piece(shifted_key("_"))),
            _ => None,
        },
        |value| KeyElement::Piece(InputPiece::Character(value)),
    )
}

fn parse_value(value: &str) -> Vec<ValueElement> {
    parse_elements(
        value,
        |token| match token {
            ANY_CHARACTER => Some(ValueElement::Any),
            LEFT_BRACKET => Some(ValueElement::Character("{".into())),
            RIGHT_BRACKET => Some(ValueElement::Character("}".into())),
            _ => None,
        },
        ValueElement::Character,
    )
}

fn parse_elements<T>(
    value: &str,
    parse_token: impl Fn(&str) -> Option<T>,
    parse_grapheme: impl Fn(String) -> T,
) -> Vec<T> {
    let mut output = Vec::new();
    let mut remaining = value;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix('{')
            && let Some(close) = rest.find('}')
            && let Some(element) = parse_token(&rest[..close])
        {
            output.push(element);
            remaining = &rest[close + 1..];
            continue;
        }
        let grapheme = graphemes(remaining)
            .into_iter()
            .next()
            .expect("nonempty text");
        remaining = &remaining[grapheme.len()..];
        output.push(parse_grapheme(grapheme));
    }
    output
}

fn shifted_key(value: &str) -> InputPiece {
    InputPiece::Key {
        intention: Some(value.into()),
        input: value.into(),
        modifiers: BTreeSet::from([InputModifier::Shift]),
    }
}

fn encode_character(value: &str) -> String {
    match value {
        "{" => format!("{{{LEFT_BRACKET}}}"),
        "}" => format!("{{{RIGHT_BRACKET}}}"),
        _ => value.to_owned(),
    }
}

fn encode_key(element: &KeyElement) -> Result<String, InputTableExportError> {
    match element {
        KeyElement::Piece(InputPiece::Character(value)) => Ok(encode_character(value)),
        KeyElement::Piece(InputPiece::CompositionSeparator) => {
            Ok(format!("{{{COMPOSITION_SEPARATOR}}}"))
        }
        KeyElement::Piece(InputPiece::Key {
            intention,
            input,
            modifiers,
        }) if intention.as_deref() == Some("0")
            && input == "0"
            && modifiers == &BTreeSet::from([InputModifier::Shift]) =>
        {
            Ok(format!("{{{SHIFT_ZERO}}}"))
        }
        KeyElement::Piece(InputPiece::Key {
            intention,
            input,
            modifiers,
        }) if intention.as_deref() == Some("_")
            && input == "_"
            && modifiers == &BTreeSet::from([InputModifier::Shift]) =>
        {
            Ok(format!("{{{SHIFT_UNDERSCORE}}}"))
        }
        KeyElement::Any => Ok(format!("{{{ANY_CHARACTER}}}")),
        _ => Err(InputTableExportError {
            element: element.clone(),
        }),
    }
}

fn encode_value(element: &ValueElement) -> String {
    match element {
        ValueElement::Character(value) => encode_character(value),
        ValueElement::Any => format!("{{{ANY_CHARACTER}}}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_exports_all_supported_tokens_in_order() {
        let source = concat!(
            "ka\tか\n",
            "n{any character}\tん{any character}\n",
            "n{composition-separator}\tん\n",
            "{lbracket}{rbracket}\t{lbracket}{rbracket}\n",
            "{shift 0}\tを\n",
            "{shift _}\tろ",
        );

        let table = InputTable::from_custom_tsv(source);

        assert_eq!(table.to_custom_tsv().unwrap(), source);
    }

    #[test]
    fn applies_a_registered_custom_table_to_composing_text() {
        use crate::{ComposingText, InputStyle, InputTableId, InputTableRegistry};

        let mut registry = InputTableRegistry::new();
        registry.register("greeting", InputTable::from_custom_tsv("aisatu\tあいさつ"));
        let mut text = ComposingText::new();
        text.insert_str(
            "aisatu",
            InputStyle::Mapped(InputTableId::Named("greeting".into())),
            &registry,
        );

        assert_eq!(text.surface(), "あいさつ");
    }

    #[test]
    fn reports_format_errors_with_zero_based_lines() {
        let source = concat!(
            "a\tあ\n",
            "a\tカ\n",
            "a{shift 0}b\tX\n",
            "{unknown}\tY\n",
            "z\t{unknown}\n",
            "broken",
        );

        let FormatReport::InvalidLines(errors) = InputTable::check_custom_tsv(source) else {
            panic!("expected invalid input-table report");
        };
        assert!(errors.contains(&FormatError {
            line: 1,
            kind: FormatErrorKind::DuplicateRule {
                first_defined_at: 0,
            },
        }));
        assert!(errors.contains(&FormatError {
            line: 2,
            kind: FormatErrorKind::ShiftTokenNotAtTail {
                token: SHIFT_ZERO.into(),
            },
        }));
        assert!(errors.contains(&FormatError {
            line: 3,
            kind: FormatErrorKind::UnknownBraceToken {
                token: "unknown".into(),
                side: FormatSide::Key,
            },
        }));
        assert!(errors.contains(&FormatError {
            line: 4,
            kind: FormatErrorKind::UnknownBraceToken {
                token: "unknown".into(),
                side: FormatSide::Value,
            },
        }));
        assert!(errors.contains(&FormatError {
            line: 5,
            kind: FormatErrorKind::InvalidTabCount { found: 0 },
        }));
    }

    #[test]
    fn rejects_unsupported_key_tokens_during_export() {
        let table = InputTable::new([(
            vec![KeyElement::Piece(InputPiece::Key {
                intention: Some("A".into()),
                input: "A".into(),
                modifiers: BTreeSet::from([InputModifier::Shift]),
            })],
            vec![ValueElement::Character("字".into())],
        )]);

        assert!(table.to_custom_tsv().is_err());
    }
}
