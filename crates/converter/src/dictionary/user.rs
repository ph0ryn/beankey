use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{DictionaryEntry, DictionaryError, DictionaryMetadata, Louds, parse_entry_shard};

const SHARD_SHIFT: usize = 11;
const SHARD_SIZE: usize = 1 << SHARD_SHIFT;

#[derive(Clone, Debug, Default)]
pub struct UserDictionary {
    entries: Vec<DictionaryEntry>,
    shortcuts: Vec<DictionaryEntry>,
}

impl UserDictionary {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, DictionaryError> {
        let root = root.into();
        Ok(Self {
            entries: load_entries(&root, "user")?,
            shortcuts: load_entries(&root, "user_shortcuts")?,
        })
    }

    pub fn entries(&self) -> &[DictionaryEntry] {
        &self.entries
    }

    pub fn shortcuts(&self) -> &[DictionaryEntry] {
        &self.shortcuts
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.shortcuts.is_empty()
    }
}

fn load_entries(root: &Path, identifier: &str) -> Result<Vec<DictionaryEntry>, DictionaryError> {
    let bits_path = root.join(format!("{identifier}.louds"));
    let chars_path = root.join(format!("{identifier}.loudschars2"));
    let (Some(bits), Some(chars)) = (read_optional(&bits_path)?, read_optional(&chars_path)?)
    else {
        return Ok(Vec::new());
    };
    let louds = Louds::parse(&bits, &chars)?;
    let mut output = Vec::new();
    for shard in 0..louds.node_count().div_ceil(SHARD_SIZE) {
        let path = root.join(format!("{identifier}{shard}.loudstxt3"));
        let Some(binary) = read_optional(&path)? else {
            continue;
        };
        let first_node = shard * SHARD_SIZE;
        let node_count = (louds.node_count() - first_node).min(SHARD_SIZE);
        output.extend(parse_entry_shard(&binary, 0..node_count)?);
    }
    for entry in &mut output {
        entry.metadata.insert(DictionaryMetadata::USER_DICTIONARY);
    }
    Ok(output)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, DictionaryError> {
    match fs::read(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DictionaryError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}
