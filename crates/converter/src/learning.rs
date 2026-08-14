use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_segmentation::UnicodeSegmentation;

use crate::lattice::is_clause;
use crate::{Candidate, DictionaryEntry, DictionaryMetadata};

const MAGIC: &[u8] = b"BEANKEY_MEMORY_V1\0";
const MEMORY_FILE: &str = "memory.bin";
const TEMPORARY_FILE: &str = "memory.bin.2";
const PAUSE_FILE: &str = ".pause";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LearningMode {
    InputAndOutput,
    OnlyOutput,
    #[default]
    Nothing,
}

impl LearningMode {
    fn uses_memory(self) -> bool {
        self != Self::Nothing
    }

    fn updates_memory(self) -> bool {
        self == Self::InputAndOutput
    }
}

#[derive(Debug)]
pub enum LearningError {
    Io { path: PathBuf, source: io::Error },
    InvalidData(&'static str),
    Poisoned,
}

impl fmt::Display for LearningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to access {}: {source}", path.display())
            }
            Self::InvalidData(reason) => write!(formatter, "invalid learning memory: {reason}"),
            Self::Poisoned => write!(formatter, "learning memory lock is poisoned"),
        }
    }
}

impl Error for LearningError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidData(_) | Self::Poisoned => None,
        }
    }
}

#[derive(Clone)]
pub struct LearningMemory {
    inner: Arc<Mutex<LearningState>>,
}

struct LearningState {
    directory: PathBuf,
    mode: LearningMode,
    max_count: usize,
    today: u16,
    persisted: Vec<LearningRecord>,
    temporary: Vec<LearningRecord>,
}

#[derive(Clone)]
struct LearningRecord {
    entry: DictionaryEntry,
    last_used_day: u16,
    last_updated_day: u16,
    count: u8,
}

impl LearningMemory {
    pub fn open(
        directory: impl Into<PathBuf>,
        mode: LearningMode,
        max_count: usize,
    ) -> Result<Self, LearningError> {
        Self::open_on_day(directory.into(), mode, max_count, current_day())
    }

    fn open_on_day(
        directory: PathBuf,
        mode: LearningMode,
        max_count: usize,
        today: u16,
    ) -> Result<Self, LearningError> {
        fs::create_dir_all(&directory).map_err(|source| LearningError::Io {
            path: directory.clone(),
            source,
        })?;
        recover_if_paused(&directory)?;
        let path = directory.join(MEMORY_FILE);
        let mut persisted = match fs::read(&path) {
            Ok(bytes) => decode(&bytes)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(source) => return Err(LearningError::Io { path, source }),
        };
        decay(&mut persisted, today);
        Ok(Self {
            inner: Arc::new(Mutex::new(LearningState {
                directory,
                mode,
                max_count,
                today,
                persisted,
                temporary: Vec::new(),
            })),
        })
    }

    pub fn entries(&self) -> Result<Vec<DictionaryEntry>, LearningError> {
        let state = self.lock()?;
        if !state.mode.uses_memory() {
            return Ok(Vec::new());
        }
        let mut output = state
            .persisted
            .iter()
            .chain(&state.temporary)
            .map(learned_entry)
            .collect::<Vec<_>>();
        deduplicate_entries(&mut output);
        Ok(output)
    }

    pub fn learn(&self, candidate: &Candidate) -> Result<(), LearningError> {
        let mut state = self.lock()?;
        if !state.mode.updates_memory() {
            return Ok(());
        }
        let today = state.today;
        for entry in candidate
            .entries
            .iter()
            .filter(|entry| learns_individual_word(entry))
        {
            memorize(&mut state.temporary, entry.clone(), today);
        }
        for entry in learned_clause_entries(&candidate.entries) {
            memorize(&mut state.temporary, entry, today);
        }
        if candidate.entries.len() > 1 {
            memorize(
                &mut state.temporary,
                join_entries(&candidate.entries),
                today,
            );
        }
        Ok(())
    }

    pub fn forget(&self, candidate: &Candidate) -> Result<(), LearningError> {
        let mut state = self.lock()?;
        if !state.mode.updates_memory() {
            return Ok(());
        }
        let targets: std::collections::HashSet<_> = candidate
            .entries
            .iter()
            .map(|entry| (entry.ruby.clone(), entry.word.clone()))
            .collect();
        state.temporary.retain(|record| {
            !targets.contains(&(record.entry.ruby.clone(), record.entry.word.clone()))
        });
        state.persisted.retain(|record| {
            !targets.contains(&(record.entry.ruby.clone(), record.entry.word.clone()))
        });
        save_locked(&mut state)
    }

    pub fn commit(&self) -> Result<bool, LearningError> {
        let mut state = self.lock()?;
        if !state.mode.updates_memory() || state.temporary.is_empty() {
            return Ok(false);
        }
        let temporary = std::mem::take(&mut state.temporary);
        for record in temporary {
            merge_record(&mut state.persisted, record);
        }
        save_locked(&mut state)?;
        Ok(true)
    }

    pub fn reset(&self) -> Result<(), LearningError> {
        let mut state = self.lock()?;
        state.persisted.clear();
        state.temporary.clear();
        for file in [MEMORY_FILE, TEMPORARY_FILE, PAUSE_FILE] {
            let path = state.directory.join(file);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(LearningError::Io { path, source }),
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, LearningState>, LearningError> {
        self.inner.lock().map_err(|_| LearningError::Poisoned)
    }
}

fn learns_individual_word(entry: &DictionaryEntry) -> bool {
    let left = entry.left_id;
    !(147..=554).contains(&left)
        && !(557..=560).contains(&left)
        && !(1297..=1305).contains(&left)
        && !(6..=9).contains(&left)
        && !matches!(left, 0 | 1316)
}

fn learned_clause_entries(entries: &[DictionaryEntry]) -> Vec<DictionaryEntry> {
    let mut clauses: Vec<Vec<DictionaryEntry>> = Vec::new();
    for entry in entries {
        if clauses.last().is_none_or(|clause| {
            clause.last().is_some_and(|previous| {
                is_clause(usize::from(previous.right_id), usize::from(entry.left_id))
            })
        }) {
            clauses.push(vec![entry.clone()]);
        } else if let Some(clause) = clauses.last_mut() {
            clause.push(entry.clone());
        }
    }
    clauses
        .windows(2)
        .map(|pair| join_entries(&pair.concat()))
        .collect()
}

fn join_entries(entries: &[DictionaryEntry]) -> DictionaryEntry {
    let first = entries.first().expect("joined entries are nonempty");
    let last = entries.last().expect("joined entries are nonempty");
    DictionaryEntry {
        word: entries.iter().map(|entry| entry.word.as_str()).collect(),
        ruby: entries.iter().map(|entry| entry.ruby.as_str()).collect(),
        left_id: first.left_id,
        right_id: last.right_id,
        meaning_id: last.meaning_id,
        base_value: entries.iter().map(|entry| entry.base_value).sum(),
        adjustment: 0.0,
        metadata: DictionaryMetadata::default(),
    }
}

fn memorize(records: &mut Vec<LearningRecord>, entry: DictionaryEntry, today: u16) {
    if let Some(record) = records
        .iter_mut()
        .find(|record| same_entry(&record.entry, &entry))
    {
        record.count = record.count.saturating_add(1);
        record.last_used_day = today;
        record.entry = entry;
    } else {
        records.push(LearningRecord {
            entry,
            last_used_day: today,
            last_updated_day: today,
            count: 1,
        });
    }
}

fn merge_record(records: &mut Vec<LearningRecord>, incoming: LearningRecord) {
    if let Some(record) = records
        .iter_mut()
        .find(|record| same_entry(&record.entry, &incoming.entry))
    {
        record.count = record.count.saturating_add(incoming.count);
        record.last_used_day = record.last_used_day.max(incoming.last_used_day);
        record.last_updated_day = record.last_updated_day.max(incoming.last_updated_day);
        record.entry = incoming.entry;
    } else {
        records.push(incoming);
    }
}

fn same_entry(left: &DictionaryEntry, right: &DictionaryEntry) -> bool {
    left.ruby == right.ruby
        && left.word == right.word
        && left.left_id == right.left_id
        && left.right_id == right.right_id
}

fn learned_entry(record: &LearningRecord) -> DictionaryEntry {
    let mut entry = record.entry.clone();
    let learned_value = learning_value(record.count, &entry.ruby);
    entry.adjustment = learned_value - entry.base_value;
    entry.metadata.insert(DictionaryMetadata::LEARNED);
    entry
}

fn learning_value(count: u8, ruby: &str) -> f32 {
    let length = UnicodeSegmentation::graphemes(ruby, true).count().max(1) as f64;
    let distance = 1.0 - f64::from(count) / 255.0;
    (-1.0 - 4.0 / length - 3.0 * distance.powi(3)) as f32
}

fn decay(records: &mut Vec<LearningRecord>, today: u16) {
    records.retain_mut(|record| {
        if today < record.last_used_day || today < record.last_updated_day {
            record.last_used_day = today;
            record.last_updated_day = today;
            record.count = 1;
        }
        if today.saturating_sub(record.last_used_day) >= 128 {
            return false;
        }
        while today.saturating_sub(record.last_updated_day) > 32 {
            record.count >>= 1;
            record.last_updated_day = record.last_updated_day.saturating_add(32);
        }
        record.count > 0
    });
}

fn deduplicate_entries(entries: &mut Vec<DictionaryEntry>) {
    let mut output = Vec::new();
    for entry in std::mem::take(entries) {
        if let Some(existing) = output
            .iter_mut()
            .find(|existing| same_entry(existing, &entry))
        {
            if existing.value() < entry.value() {
                *existing = entry;
            }
        } else {
            output.push(entry);
        }
    }
    *entries = output;
}

fn save_locked(state: &mut LearningState) -> Result<(), LearningError> {
    decay(&mut state.persisted, state.today);
    state
        .persisted
        .sort_by_key(|record| std::cmp::Reverse(record.last_used_day));
    state.persisted.truncate(state.max_count);
    let bytes = encode(&state.persisted);
    let temporary = state.directory.join(TEMPORARY_FILE);
    fs::write(&temporary, bytes).map_err(|source| LearningError::Io {
        path: temporary.clone(),
        source,
    })?;
    let pause = state.directory.join(PAUSE_FILE);
    fs::write(&pause, []).map_err(|source| LearningError::Io {
        path: pause.clone(),
        source,
    })?;
    let memory = state.directory.join(MEMORY_FILE);
    fs::copy(&temporary, &memory).map_err(|source| LearningError::Io {
        path: memory,
        source,
    })?;
    fs::remove_file(&pause).map_err(|source| LearningError::Io {
        path: pause,
        source,
    })?;
    Ok(())
}

fn recover_if_paused(directory: &Path) -> Result<(), LearningError> {
    let pause = directory.join(PAUSE_FILE);
    if !pause.exists() {
        return Ok(());
    }
    let temporary = directory.join(TEMPORARY_FILE);
    let memory = directory.join(MEMORY_FILE);
    fs::copy(&temporary, &memory).map_err(|source| LearningError::Io {
        path: memory,
        source,
    })?;
    fs::remove_file(&pause).map_err(|source| LearningError::Io {
        path: pause,
        source,
    })?;
    Ok(())
}

fn encode(records: &[LearningRecord]) -> Vec<u8> {
    let mut output = MAGIC.to_vec();
    output.extend(
        u32::try_from(records.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for record in records {
        output.extend(record.last_used_day.to_le_bytes());
        output.extend(record.last_updated_day.to_le_bytes());
        output.push(record.count);
        write_string(&mut output, &record.entry.word);
        write_string(&mut output, &record.entry.ruby);
        output.extend(record.entry.left_id.to_le_bytes());
        output.extend(record.entry.right_id.to_le_bytes());
        output.extend(record.entry.meaning_id.to_le_bytes());
        output.extend(record.entry.base_value.to_le_bytes());
    }
    output
}

fn decode(bytes: &[u8]) -> Result<Vec<LearningRecord>, LearningError> {
    if !bytes.starts_with(MAGIC) {
        return Err(LearningError::InvalidData("bad magic"));
    }
    let mut cursor = MAGIC.len();
    let count = read_u32(bytes, &mut cursor)? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let last_used_day = read_u16(bytes, &mut cursor)?;
        let last_updated_day = read_u16(bytes, &mut cursor)?;
        let count = *bytes
            .get(cursor)
            .ok_or(LearningError::InvalidData("truncated count"))?;
        cursor += 1;
        let word = read_string(bytes, &mut cursor)?;
        let ruby = read_string(bytes, &mut cursor)?;
        let left_id = read_u16(bytes, &mut cursor)?;
        let right_id = read_u16(bytes, &mut cursor)?;
        let meaning_id = read_u16(bytes, &mut cursor)?;
        let base_value = f32::from_le_bytes(read_array(bytes, &mut cursor)?);
        records.push(LearningRecord {
            entry: DictionaryEntry {
                word,
                ruby,
                left_id,
                right_id,
                meaning_id,
                base_value,
                adjustment: 0.0,
                metadata: DictionaryMetadata::default(),
            },
            last_used_day,
            last_updated_day,
            count,
        });
    }
    if cursor != bytes.len() {
        return Err(LearningError::InvalidData("trailing bytes"));
    }
    Ok(records)
}

fn write_string(output: &mut Vec<u8>, value: &str) {
    output.extend(u32::try_from(value.len()).unwrap_or(u32::MAX).to_le_bytes());
    output.extend(value.as_bytes());
}

fn read_string(bytes: &[u8], cursor: &mut usize) -> Result<String, LearningError> {
    let length = read_u32(bytes, cursor)? as usize;
    let value = bytes
        .get(*cursor..cursor.saturating_add(length))
        .ok_or(LearningError::InvalidData("truncated string"))?;
    *cursor += length;
    String::from_utf8(value.to_vec()).map_err(|_| LearningError::InvalidData("invalid UTF-8"))
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, LearningError> {
    Ok(u16::from_le_bytes(read_array(bytes, cursor)?))
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, LearningError> {
    Ok(u32::from_le_bytes(read_array(bytes, cursor)?))
}

fn read_array<const N: usize>(bytes: &[u8], cursor: &mut usize) -> Result<[u8; N], LearningError> {
    let value = bytes
        .get(*cursor..cursor.saturating_add(N))
        .ok_or(LearningError::InvalidData("truncated numeric field"))?;
    *cursor += N;
    value
        .try_into()
        .map_err(|_| LearningError::InvalidData("invalid numeric field"))
}

fn current_day() -> u16 {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    u16::try_from(days.saturating_sub(19_000)).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(word: &str, ruby: &str) -> DictionaryEntry {
        DictionaryEntry {
            word: word.into(),
            ruby: ruby.into(),
            left_id: 1285,
            right_id: 1285,
            meaning_id: 501,
            base_value: -12.0,
            adjustment: 0.0,
            metadata: DictionaryMetadata::default(),
        }
    }

    #[test]
    fn halves_counts_after_each_complete_thirty_two_day_period() {
        let mut records = vec![LearningRecord {
            entry: entry("語", "ゴ"),
            last_used_day: 100,
            last_updated_day: 100,
            count: 4,
        }];
        decay(&mut records, 165);
        assert_eq!(records[0].count, 1);
        decay(&mut records, 197);
        assert!(records.is_empty());
    }

    #[test]
    fn round_trips_the_private_persistence_format() {
        let records = vec![LearningRecord {
            entry: entry("単語", "タンゴ"),
            last_used_day: 10,
            last_updated_day: 9,
            count: 3,
        }];
        let decoded = decode(&encode(&records)).unwrap();
        assert_eq!(decoded[0].entry.word, "単語");
        assert_eq!(decoded[0].count, 3);
    }
}
