use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::kana::to_katakana;
use crate::zenz::tokenize_for_model;
use crate::{
    ComposingText, InputPiece, InputStyle, InputTable, InputTableRegistry, ZenzInferenceError,
    ZenzLanguageModel, ZenzPromptBuilder,
};

#[derive(Clone, Debug, PartialEq)]
pub struct LmTypoConfig {
    pub beam_size: usize,
    pub top_k: usize,
    pub n_best: usize,
    pub max_steps: Option<usize>,
    pub substitution_cost: f32,
    pub deletion_cost: f32,
    pub transposition_cost: f32,
}

impl Default for LmTypoConfig {
    fn default() -> Self {
        Self {
            beam_size: 32,
            top_k: 64,
            n_best: 5,
            max_steps: None,
            substitution_cost: 2.0,
            deletion_cost: 3.0,
            transposition_cost: 2.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LmTypoCandidate {
    pub corrected_input: String,
    pub converted_text: String,
    pub score: f32,
    pub lm_score: f32,
    pub channel_cost: f32,
    pub prominence: f32,
}

#[derive(Clone)]
struct Hypothesis {
    corrected: Vec<String>,
    observed_position: usize,
    previous_input: Option<String>,
    channel_cost: f32,
    lm_score: f32,
    score: f32,
}

#[derive(Clone, Copy)]
enum Topology {
    MacQwerty,
    IosQwerty,
    IosFlickTenkey,
}

struct GenerationMode<'a> {
    table: Option<&'a InputTable>,
    topology: Topology,
    use_surface: bool,
    filter_with_lm: bool,
}

pub fn experimental_typo_correction(
    model: &mut dyn ZenzLanguageModel,
    left_context: &str,
    composing: &ComposingText,
    input_style: &InputStyle,
    tables: &InputTableRegistry,
    config: &LmTypoConfig,
) -> Result<Vec<LmTypoCandidate>, ZenzInferenceError> {
    if config.n_best == 0 || config.beam_size == 0 {
        return Ok(Vec::new());
    }
    let mode = generation_mode(input_style, tables);
    let observed = observed_input(composing, mode.use_surface);
    if observed.is_empty() {
        return Ok(Vec::new());
    }

    let prompt = ZenzPromptBuilder::typo_correction_prefix(left_context);
    let prompt_tokens = tokenize_for_model(model, &prompt, false)?;
    let mut score_cache = HashMap::<String, f32>::new();
    let mut beam = vec![Hypothesis {
        corrected: Vec::new(),
        observed_position: 0,
        previous_input: None,
        channel_cost: 0.0,
        lm_score: 0.0,
        score: 0.0,
    }];
    let max_steps = config.max_steps.unwrap_or(observed.len() * 2 + 8);

    for _ in 0..max_steps {
        if beam
            .iter()
            .all(|hypothesis| hypothesis.observed_position == observed.len())
        {
            break;
        }
        let mut expanded = Vec::new();
        for hypothesis in &beam {
            if hypothesis.observed_position == observed.len() {
                expanded.push(hypothesis.clone());
                continue;
            }
            expand_hypothesis(
                model,
                &prompt_tokens,
                &observed,
                hypothesis,
                &mode,
                input_style,
                tables,
                config,
                &mut score_cache,
                &mut expanded,
            )?;
        }
        if expanded.is_empty() {
            break;
        }
        expanded.sort_by(compare_hypotheses);
        expanded.truncate(config.beam_size);
        beam = expanded;
    }

    for hypothesis in &mut beam {
        if hypothesis.observed_position < observed.len() {
            hypothesis
                .corrected
                .extend_from_slice(&observed[hypothesis.observed_position..]);
            hypothesis.previous_input = hypothesis.corrected.last().cloned();
            hypothesis.observed_position = observed.len();
            score_hypothesis(
                model,
                &prompt_tokens,
                hypothesis,
                input_style,
                tables,
                &mut score_cache,
            )?;
        }
    }

    let mut original = Hypothesis {
        corrected: observed,
        observed_position: 0,
        previous_input: None,
        channel_cost: 0.0,
        lm_score: 0.0,
        score: 0.0,
    };
    original.observed_position = original.corrected.len();
    original.previous_input = original.corrected.last().cloned();
    score_hypothesis(
        model,
        &prompt_tokens,
        &mut original,
        input_style,
        tables,
        &mut score_cache,
    )?;
    beam.push(original);

    let mut unique = HashMap::<String, LmTypoCandidate>::new();
    for hypothesis in beam {
        let corrected_input = hypothesis.corrected.concat();
        let converted_text = converted_text(&corrected_input, input_style, tables);
        let candidate = LmTypoCandidate {
            corrected_input: corrected_input.clone(),
            converted_text,
            score: hypothesis.score,
            lm_score: hypothesis.lm_score,
            channel_cost: hypothesis.channel_cost,
            prominence: 0.0,
        };
        if unique
            .get(&corrected_input)
            .is_none_or(|existing| existing.score < candidate.score)
        {
            unique.insert(corrected_input, candidate);
        }
    }
    let mut candidates: Vec<_> = unique.into_values().collect();
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.corrected_input.cmp(&right.corrected_input))
    });
    candidates.truncate(config.n_best);
    if let Some(best) = candidates.first().map(|candidate| candidate.score) {
        for candidate in &mut candidates {
            candidate.prominence = (candidate.score - best).exp();
        }
    }
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn expand_hypothesis(
    model: &mut dyn ZenzLanguageModel,
    prompt_tokens: &[i32],
    observed: &[String],
    hypothesis: &Hypothesis,
    mode: &GenerationMode<'_>,
    input_style: &InputStyle,
    tables: &InputTableRegistry,
    config: &LmTypoConfig,
    score_cache: &mut HashMap<String, f32>,
    expanded: &mut Vec<Hypothesis>,
) -> Result<(), ZenzInferenceError> {
    let position = hypothesis.observed_position;
    let current = &observed[position];
    let mut targets = vec![(current.clone(), 0.0)];
    let mut substitutions = neighbors(current, mode.topology);
    if mode.filter_with_lm {
        let converted_prefix = converted_text(&hypothesis.corrected.concat(), input_style, tables);
        let likely = top_characters(model, prompt_tokens, &converted_prefix, config.top_k)?;
        substitutions.retain(|(candidate, _)| likely.contains(candidate));
    }
    targets.extend(
        substitutions
            .into_iter()
            .map(|(target, distance)| (target, config.substitution_cost * distance)),
    );

    for (target, channel_addition) in targets {
        let mut next = hypothesis.clone();
        next.corrected.push(target.clone());
        next.observed_position += 1;
        next.previous_input = Some(target);
        next.channel_cost += channel_addition;
        if tail_pending_is_modified(&next, observed, mode, channel_addition) {
            continue;
        }
        score_hypothesis(
            model,
            prompt_tokens,
            &mut next,
            input_style,
            tables,
            score_cache,
        )?;
        expanded.push(next);
    }

    if position + 1 < observed.len()
        && hypothesis.previous_input.as_ref().is_some_and(|previous| {
            neighbors(previous, mode.topology)
                .iter()
                .any(|(neighbor, _)| neighbor == current)
        })
    {
        let distance = neighbors(
            hypothesis.previous_input.as_deref().unwrap_or_default(),
            mode.topology,
        )
        .into_iter()
        .find_map(|(neighbor, distance)| (neighbor == *current).then_some(distance))
        .unwrap_or(1.0);
        let mut next = hypothesis.clone();
        next.observed_position += 1;
        next.channel_cost += config.deletion_cost * distance;
        score_hypothesis(
            model,
            prompt_tokens,
            &mut next,
            input_style,
            tables,
            score_cache,
        )?;
        expanded.push(next);
    }

    if position + 1 < observed.len() && observed[position] != observed[position + 1] {
        let mut next = hypothesis.clone();
        next.corrected.push(observed[position + 1].clone());
        next.corrected.push(observed[position].clone());
        next.observed_position += 2;
        next.previous_input = Some(observed[position].clone());
        next.channel_cost += config.transposition_cost;
        if tail_pending_is_modified(&next, observed, mode, config.transposition_cost) {
            return Ok(());
        }
        score_hypothesis(
            model,
            prompt_tokens,
            &mut next,
            input_style,
            tables,
            score_cache,
        )?;
        expanded.push(next);
    }
    Ok(())
}

fn score_hypothesis(
    model: &mut dyn ZenzLanguageModel,
    prompt_tokens: &[i32],
    hypothesis: &mut Hypothesis,
    input_style: &InputStyle,
    tables: &InputTableRegistry,
    score_cache: &mut HashMap<String, f32>,
) -> Result<(), ZenzInferenceError> {
    let corrected_input = hypothesis.corrected.concat();
    let converted = converted_text(&corrected_input, input_style, tables);
    hypothesis.lm_score = if let Some(score) = score_cache.get(&converted) {
        *score
    } else {
        let score = language_model_score(model, prompt_tokens, &converted)?;
        score_cache.insert(converted, score);
        score
    };
    hypothesis.score = hypothesis.lm_score - hypothesis.channel_cost;
    Ok(())
}

fn compare_hypotheses(left: &Hypothesis, right: &Hypothesis) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.corrected.cmp(&right.corrected))
}

fn language_model_score(
    model: &mut dyn ZenzLanguageModel,
    prompt_tokens: &[i32],
    text: &str,
) -> Result<f32, ZenzInferenceError> {
    let tokens = tokenize_for_model(model, text, false)?;
    let mut prefix = prompt_tokens.to_vec();
    let mut score = 0.0;
    for token in tokens {
        let log_probabilities = next_log_probabilities(model, &prefix)?;
        let index = usize::try_from(token)
            .ok()
            .filter(|index| *index < log_probabilities.len())
            .ok_or_else(|| ZenzInferenceError(format!("invalid typo candidate token {token}")))?;
        score += log_probabilities[index];
        prefix.push(token);
    }
    Ok(score)
}

fn next_log_probabilities(
    model: &mut dyn ZenzLanguageModel,
    tokens: &[i32],
) -> Result<Vec<f32>, ZenzInferenceError> {
    let logits = model.next_logits(tokens)?;
    if logits.len() != model.vocabulary_size() || logits.is_empty() {
        return Err(ZenzInferenceError(format!(
            "model returned {} logits for a vocabulary of {}",
            logits.len(),
            model.vocabulary_size()
        )));
    }
    let maximum = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum = logits
        .iter()
        .map(|value| (*value - maximum).exp())
        .sum::<f32>();
    let normalization = maximum + sum.ln();
    Ok(logits
        .into_iter()
        .map(|value| value - normalization)
        .collect())
}

fn top_characters(
    model: &mut dyn ZenzLanguageModel,
    prompt_tokens: &[i32],
    converted_prefix: &str,
    count: usize,
) -> Result<Vec<String>, ZenzInferenceError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut tokens = prompt_tokens.to_vec();
    tokens.extend(tokenize_for_model(model, converted_prefix, false)?);
    let probabilities = next_log_probabilities(model, &tokens)?;
    let mut token_ids: Vec<_> = (0..probabilities.len()).collect();
    token_ids.sort_by(|left, right| probabilities[*right].total_cmp(&probabilities[*left]));
    let mut result = Vec::new();
    for token_id in token_ids.into_iter().take(count.saturating_mul(4)) {
        let Ok(token_id) = i32::try_from(token_id) else {
            continue;
        };
        let Ok(piece) = model.token_to_piece(token_id) else {
            continue;
        };
        let Ok(piece) = String::from_utf8(piece) else {
            continue;
        };
        let graphemes: Vec<_> = UnicodeSegmentation::graphemes(piece.as_str(), true).collect();
        if graphemes.len() == 1 && !result.iter().any(|existing| existing == graphemes[0]) {
            result.push(graphemes[0].to_owned());
            if result.len() == count {
                break;
            }
        }
    }
    Ok(result)
}

fn generation_mode<'a>(
    input_style: &InputStyle,
    tables: &'a InputTableRegistry,
) -> GenerationMode<'a> {
    match input_style {
        InputStyle::RomanToKana => GenerationMode {
            table: tables.resolve(input_style),
            topology: Topology::MacQwerty,
            use_surface: false,
            filter_with_lm: false,
        },
        InputStyle::Mapped(_) => {
            let table = tables.resolve(input_style);
            let qwerty = table.is_some_and(|table| !table.possible_nexts("").is_empty());
            GenerationMode {
                table,
                topology: if qwerty {
                    Topology::IosQwerty
                } else {
                    Topology::IosFlickTenkey
                },
                use_surface: !qwerty,
                filter_with_lm: !qwerty,
            }
        }
        InputStyle::Direct => GenerationMode {
            table: None,
            topology: Topology::IosFlickTenkey,
            use_surface: true,
            filter_with_lm: true,
        },
    }
}

fn observed_input(composing: &ComposingText, use_surface: bool) -> Vec<String> {
    if use_surface {
        return UnicodeSegmentation::graphemes(to_katakana(&composing.surface()).as_str(), true)
            .map(str::to_owned)
            .collect();
    }
    composing
        .input()
        .iter()
        .filter_map(|element| match &element.piece {
            InputPiece::Character(character) => {
                character.graphemes(true).next().map(str::to_lowercase)
            }
            InputPiece::Key {
                intention, input, ..
            } => intention
                .as_deref()
                .unwrap_or(input)
                .graphemes(true)
                .next()
                .map(str::to_lowercase),
            InputPiece::CompositionSeparator => None,
        })
        .collect()
}

fn tail_pending_is_modified(
    hypothesis: &Hypothesis,
    observed: &[String],
    mode: &GenerationMode<'_>,
    channel_addition: f32,
) -> bool {
    if hypothesis.observed_position != observed.len() {
        return false;
    }
    let Some(table) = mode.table else {
        return false;
    };
    let corrected = hypothesis.corrected.concat();
    let last_was_modified = hypothesis.corrected.last() != observed.last();
    trailing_pending(&corrected, table).is_some() && (last_was_modified || channel_addition > 0.0)
}

fn trailing_pending<'a>(input: &'a str, table: &InputTable) -> Option<&'a str> {
    let boundaries: Vec<_> = input.char_indices().map(|(index, _)| index).collect();
    for start in boundaries {
        let suffix = &input[start..];
        if !table.possible_nexts(suffix).is_empty() && apply_input_table(suffix, table) == suffix {
            return Some(suffix);
        }
    }
    None
}

fn apply_input_table(input: &str, table: &InputTable) -> String {
    input.graphemes(true).fold(String::new(), |current, piece| {
        table.applied(&current, InputPiece::character(piece))
    })
}

fn converted_text(
    corrected_input: &str,
    input_style: &InputStyle,
    tables: &InputTableRegistry,
) -> String {
    if matches!(input_style, InputStyle::Direct) {
        return corrected_input.to_owned();
    }
    let mut composing = ComposingText::new();
    composing.insert_str(corrected_input, input_style.clone(), tables);
    to_katakana(&composing.surface())
}

fn neighbors(character: &str, topology: Topology) -> Vec<(String, f32)> {
    match topology {
        Topology::MacQwerty => coordinate_neighbors(character, MAC_QWERTY),
        Topology::IosQwerty => coordinate_neighbors(character, IOS_QWERTY),
        Topology::IosFlickTenkey => tenkey_neighbors(character),
    }
}

fn coordinate_neighbors(character: &str, coordinates: &[(&str, f32, f32)]) -> Vec<(String, f32)> {
    let Some((_, x, y)) = coordinates.iter().find(|(key, _, _)| *key == character) else {
        return Vec::new();
    };
    let mut result: Vec<_> = coordinates
        .iter()
        .filter_map(|(key, target_x, target_y)| {
            if *key == character {
                return None;
            }
            let distance = ((*x - *target_x).powi(2) + (*y - *target_y).powi(2)).sqrt();
            (distance <= 1.65).then(|| ((*key).to_owned(), distance))
        })
        .collect();
    result.sort_by(|left, right| left.0.cmp(&right.0));
    result
}

fn tenkey_neighbors(character: &str) -> Vec<(String, f32)> {
    const GROUPS: [&str; 16] = [
        "アイウエオ",
        "カキクケコ",
        "ガギグゲゴ",
        "サシスセソ",
        "ザジズゼゾ",
        "タチツテト",
        "ダヂヅデド",
        "ナニヌネノ",
        "ハヒフヘホ",
        "バビブベボ",
        "パピプペポ",
        "マミムメモ",
        "ヤユヨ",
        "ャュョ",
        "ラリルレロ",
        "ワヲンー",
    ];
    GROUPS
        .iter()
        .find(|group| group.contains(character))
        .map(|group| {
            UnicodeSegmentation::graphemes(*group, true)
                .filter(|neighbor| *neighbor != character)
                .map(|neighbor| (neighbor.to_owned(), 1.0))
                .collect()
        })
        .unwrap_or_default()
}

const MAC_QWERTY: &[(&str, f32, f32)] = &[
    ("1", -1.0, 0.0),
    ("2", 0.25, 0.0),
    ("3", 1.25, 0.0),
    ("4", 2.25, 0.0),
    ("5", 3.25, 0.0),
    ("6", 4.25, 0.0),
    ("7", 5.25, 0.0),
    ("8", 6.25, 0.0),
    ("9", 7.25, 0.0),
    ("0", 8.25, 0.0),
    ("-", 9.25, 0.0),
    ("^", 10.25, 0.0),
    ("q", 0.0, 1.0),
    ("w", 1.0, 1.0),
    ("e", 2.0, 1.0),
    ("r", 3.0, 1.0),
    ("t", 4.0, 1.0),
    ("y", 5.0, 1.0),
    ("u", 6.0, 1.0),
    ("i", 7.0, 1.0),
    ("o", 8.0, 1.0),
    ("p", 9.0, 1.0),
    ("@", 10.0, 1.0),
    ("[", 11.0, 1.0),
    ("a", 0.25, 2.0),
    ("s", 1.25, 2.0),
    ("d", 2.25, 2.0),
    ("f", 3.25, 2.0),
    ("g", 4.25, 2.0),
    ("h", 5.25, 2.0),
    ("j", 6.25, 2.0),
    ("k", 7.25, 2.0),
    ("l", 8.25, 2.0),
    (";", 9.25, 2.0),
    ("]", 10.25, 2.0),
    ("z", 0.8, 3.0),
    ("x", 1.8, 3.0),
    ("c", 2.8, 3.0),
    ("v", 3.8, 3.0),
    ("b", 4.8, 3.0),
    ("n", 5.8, 3.0),
    ("m", 6.8, 3.0),
    (",", 7.8, 3.0),
    (".", 8.8, 3.0),
    ("/", 9.8, 3.0),
    ("_", 10.8, 3.0),
];

const IOS_QWERTY: &[(&str, f32, f32)] = &[
    ("q", 0.0, 1.0),
    ("w", 1.0, 1.0),
    ("e", 2.0, 1.0),
    ("r", 3.0, 1.0),
    ("t", 4.0, 1.0),
    ("y", 5.0, 1.0),
    ("u", 6.0, 1.0),
    ("i", 7.0, 1.0),
    ("o", 8.0, 1.0),
    ("p", 9.0, 1.0),
    ("a", 0.5, 2.5),
    ("s", 1.5, 2.5),
    ("d", 2.5, 2.5),
    ("f", 3.5, 2.5),
    ("g", 4.5, 2.5),
    ("h", 5.5, 2.5),
    ("j", 6.5, 2.5),
    ("k", 7.5, 2.5),
    ("l", 8.5, 2.5),
    ("z", 1.5, 4.0),
    ("x", 2.5, 4.0),
    ("c", 3.5, 4.0),
    ("v", 4.5, 4.0),
    ("b", 5.5, 4.0),
    ("n", 6.5, 4.0),
    ("m", 7.5, 4.0),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct TestModel {
        pieces: Vec<Vec<u8>>,
        token_ids: HashMap<String, i32>,
        preferred: i32,
    }

    impl TestModel {
        fn new(values: &[&str], preferred: &str) -> Self {
            let pieces: Vec<_> = values
                .iter()
                .map(|value| value.as_bytes().to_vec())
                .collect();
            let token_ids: HashMap<String, i32> = values
                .iter()
                .enumerate()
                .map(|(index, value)| (value.to_string(), index as i32))
                .collect();
            let preferred = token_ids[preferred];
            Self {
                pieces,
                token_ids,
                preferred,
            }
        }
    }

    impl ZenzLanguageModel for TestModel {
        fn vocabulary_size(&self) -> usize {
            self.pieces.len()
        }
        fn eos_token(&self) -> i32 {
            0
        }
        fn tokenize(
            &mut self,
            text: &str,
            _add_special: bool,
        ) -> Result<Vec<i32>, ZenzInferenceError> {
            Ok(text
                .graphemes(true)
                .filter_map(|piece| self.token_ids.get(piece).copied())
                .collect())
        }
        fn token_to_piece(&mut self, token: i32) -> Result<Vec<u8>, ZenzInferenceError> {
            Ok(self.pieces[token as usize].clone())
        }
        fn next_logits(&mut self, _tokens: &[i32]) -> Result<Vec<f32>, ZenzInferenceError> {
            let mut logits = vec![-8.0; self.pieces.len()];
            logits[self.preferred as usize] = 8.0;
            Ok(logits)
        }
    }

    fn composing(value: &str, style: InputStyle) -> (ComposingText, InputTableRegistry) {
        let tables = InputTableRegistry::new();
        let mut composing = ComposingText::new();
        composing.insert_str(value, style, &tables);
        (composing, tables)
    }

    #[test]
    fn direct_input_uses_zenz_to_rank_tenkey_substitutions() {
        let (composing, tables) = composing("か", InputStyle::Direct);
        let mut model = TestModel::new(&["\u{ee00}", "カ", "キ", "ク", "ケ", "コ"], "キ");
        let candidates = experimental_typo_correction(
            &mut model,
            "",
            &composing,
            &InputStyle::Direct,
            &tables,
            &LmTypoConfig::default(),
        )
        .unwrap();
        assert_eq!(candidates[0].corrected_input, "キ");
        assert_eq!(candidates[0].converted_text, "キ");
        assert_eq!(candidates[0].channel_cost, 2.0);
        assert_eq!(candidates[0].prominence, 1.0);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.corrected_input == "カ")
        );
    }

    #[test]
    fn roman_input_can_recover_an_adjacent_transposition() {
        let (composing, tables) = composing("ak", InputStyle::RomanToKana);
        let mut model = TestModel::new(&["\u{ee00}", "カ", "ア", "k", "a"], "カ");
        let config = LmTypoConfig {
            beam_size: 128,
            n_best: 64,
            transposition_cost: 0.0,
            ..LmTypoConfig::default()
        };
        let candidates = experimental_typo_correction(
            &mut model,
            "",
            &composing,
            &InputStyle::RomanToKana,
            &tables,
            &config,
        )
        .unwrap();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.corrected_input == "ka")
        );
    }

    #[test]
    fn deletion_requires_the_observed_key_to_neighbor_the_previous_key() {
        let (composing, tables) = composing("sda", InputStyle::RomanToKana);
        let mut model = TestModel::new(&["\u{ee00}", "サ", "ダ", "s", "d", "a"], "サ");
        let config = LmTypoConfig {
            beam_size: 128,
            n_best: 64,
            deletion_cost: 0.0,
            ..LmTypoConfig::default()
        };
        let candidates = experimental_typo_correction(
            &mut model,
            "",
            &composing,
            &InputStyle::RomanToKana,
            &tables,
            &config,
        )
        .unwrap();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.corrected_input == "sa")
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.corrected_input == "sd")
        );
    }

    #[test]
    fn max_steps_completes_the_unexplored_suffix_as_observed_input() {
        let (composing, tables) = composing("kana", InputStyle::RomanToKana);
        let mut model = TestModel::new(&["\u{ee00}", "カ", "ナ", "k", "a", "n"], "カ");
        let config = LmTypoConfig {
            max_steps: Some(0),
            ..LmTypoConfig::default()
        };
        let candidates = experimental_typo_correction(
            &mut model,
            "",
            &composing,
            &InputStyle::RomanToKana,
            &tables,
            &config,
        )
        .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].corrected_input, "kana");
        assert_eq!(candidates[0].channel_cost, 0.0);
    }
}
