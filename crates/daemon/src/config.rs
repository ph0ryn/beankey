use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const EXPECTED_CONTEXT_SIZE: usize = 512;
const EXPECTED_BATCH_SIZE: usize = 512;
const EXPECTED_MICRO_BATCH_SIZE: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    pub dictionary: PathBuf,
    pub model: PathBuf,
    pub emoji_dictionary: PathBuf,
    pub llama_backend_directory: PathBuf,
    pub runtime_socket: PathBuf,
    pub hunspell: HunspellConfig,
    pub conversion: ConversionConfig,
    pub zenz: ZenzConfig,
    pub inference: InferenceConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ConversionConfig {
    pub n_best: usize,
    pub japanese_prediction: PredictionConfig,
    pub foreign_prediction: PredictionConfig,
    pub full_width_roman: bool,
    pub half_width_kana: bool,
    pub typography: bool,
    pub typo_correction: TypoCorrectionConfig,
    pub live_conversion: bool,
    pub user_dictionary: Option<PathBuf>,
    pub user_dictionary_directory: Option<PathBuf>,
    pub custom_input_tables: BTreeMap<String, PathBuf>,
}

impl Default for ConversionConfig {
    fn default() -> Self {
        Self {
            n_best: 10,
            japanese_prediction: PredictionConfig::Automatic,
            foreign_prediction: PredictionConfig::Automatic,
            full_width_roman: false,
            half_width_kana: false,
            typography: false,
            typo_correction: TypoCorrectionConfig::Automatic,
            live_conversion: false,
            user_dictionary: None,
            user_dictionary_directory: None,
            custom_input_tables: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PredictionConfig {
    #[default]
    Automatic,
    Manual,
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TypoCorrectionConfig {
    Enabled,
    #[default]
    Automatic,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ZenzConfig {
    pub inference_limit: usize,
    pub rich_candidates: bool,
    pub predictive_input: bool,
    pub profile: Option<String>,
    pub topic: Option<String>,
    pub style: Option<String>,
    pub preference: Option<String>,
    pub enable_alignment_separator: bool,
    pub personalization: Option<PersonalizationConfig>,
}

impl Default for ZenzConfig {
    fn default() -> Self {
        Self {
            inference_limit: 10,
            rich_candidates: false,
            predictive_input: false,
            profile: None,
            topic: None,
            style: None,
            preference: None,
            enable_alignment_separator: false,
            personalization: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersonalizationConfig {
    pub base_ngram: PathBuf,
    pub personal_ngram: PathBuf,
    pub alpha: f32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HunspellConfig {
    pub english_dictionary: PathBuf,
    pub greek_dictionary: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InferenceConfig {
    pub context_size: usize,
    pub batch_size: usize,
    pub micro_batch_size: usize,
    pub flash_attention: bool,
}

#[derive(Debug)]
pub enum DaemonConfigError {
    Read(std::io::Error),
    Parse(toml::de::Error),
    InvalidRuntimeSocket,
    InvalidConversionCandidateCount,
    InvalidPersonalizationAlpha,
    UnsupportedInferenceProfile(InferenceConfig),
}

impl fmt::Display for DaemonConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "could not read daemon configuration: {error}"),
            Self::Parse(error) => write!(formatter, "invalid daemon configuration: {error}"),
            Self::InvalidRuntimeSocket => {
                write!(formatter, "runtime_socket must be beankey/daemon.sock")
            }
            Self::InvalidConversionCandidateCount => {
                write!(formatter, "conversion.n_best must be greater than zero")
            }
            Self::InvalidPersonalizationAlpha => {
                write!(
                    formatter,
                    "zenz.personalization.alpha must be finite and nonnegative"
                )
            }
            Self::UnsupportedInferenceProfile(profile) => write!(
                formatter,
                "unsupported inference profile: context={}, batch={}, microbatch={}, flash_attention={}",
                profile.context_size,
                profile.batch_size,
                profile.micro_batch_size,
                profile.flash_attention
            ),
        }
    }
}

impl Error for DaemonConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::InvalidRuntimeSocket
            | Self::InvalidConversionCandidateCount
            | Self::InvalidPersonalizationAlpha
            | Self::UnsupportedInferenceProfile(_) => None,
        }
    }
}

impl DaemonConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DaemonConfigError> {
        let source = fs::read_to_string(path).map_err(DaemonConfigError::Read)?;
        Self::parse(&source)
    }

    pub fn parse(source: &str) -> Result<Self, DaemonConfigError> {
        let config: Self = toml::from_str(source).map_err(DaemonConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), DaemonConfigError> {
        if self.runtime_socket != Path::new("beankey/daemon.sock")
            || self.runtime_socket.components().any(|component| {
                matches!(
                    component,
                    Component::Prefix(_) | Component::RootDir | Component::ParentDir
                )
            })
        {
            return Err(DaemonConfigError::InvalidRuntimeSocket);
        }
        if self.inference.context_size != EXPECTED_CONTEXT_SIZE
            || self.inference.batch_size != EXPECTED_BATCH_SIZE
            || self.inference.micro_batch_size != EXPECTED_MICRO_BATCH_SIZE
            || !self.inference.flash_attention
        {
            return Err(DaemonConfigError::UnsupportedInferenceProfile(
                self.inference.clone(),
            ));
        }
        if self.conversion.n_best == 0 {
            return Err(DaemonConfigError::InvalidConversionCandidateCount);
        }
        if self
            .zenz
            .personalization
            .as_ref()
            .is_some_and(|config| !config.alpha.is_finite() || config.alpha < 0.0)
        {
            return Err(DaemonConfigError::InvalidPersonalizationAlpha);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
dictionary = "/nix/store/dictionary"
model = "/nix/store/model.gguf"
emoji_dictionary = "/nix/store/emoji_all_E17.0.txt"
llama_backend_directory = "/nix/store/llama/bin"
runtime_socket = "beankey/daemon.sock"

[hunspell]
english_dictionary = "/nix/store/en_US"
greek_dictionary = "/nix/store/el_GR"

[conversion]
n_best = 10
japanese_prediction = "automatic"
foreign_prediction = "automatic"
full_width_roman = false
half_width_kana = false
typography = false
typo_correction = "automatic"
live_conversion = false
custom_input_tables = {}

[zenz]
inference_limit = 10
rich_candidates = false
predictive_input = false
enable_alignment_separator = false

[inference]
context_size = 512
batch_size = 512
micro_batch_size = 64
flash_attention = true
"#;

    #[test]
    fn parses_the_internal_nixos_configuration() {
        let config = DaemonConfig::parse(CONFIG).unwrap();
        assert_eq!(config.runtime_socket, Path::new("beankey/daemon.sock"));
        assert_eq!(config.inference.context_size, 512);
        assert_eq!(config.zenz.inference_limit, 10);
    }

    #[test]
    fn rejects_path_traversal_and_noncanonical_inference_profiles() {
        let traversal = CONFIG.replace(
            "runtime_socket = \"beankey/daemon.sock\"",
            "runtime_socket = \"../daemon.sock\"",
        );
        assert!(matches!(
            DaemonConfig::parse(&traversal),
            Err(DaemonConfigError::InvalidRuntimeSocket)
        ));

        let profile = CONFIG.replace("context_size = 512", "context_size = 1024");
        assert!(matches!(
            DaemonConfig::parse(&profile),
            Err(DaemonConfigError::UnsupportedInferenceProfile(_))
        ));
    }

    #[test]
    fn rejects_unknown_internal_settings() {
        let unknown = format!("{CONFIG}\nunexpected = true\n");
        assert!(matches!(
            DaemonConfig::parse(&unknown),
            Err(DaemonConfigError::Parse(_))
        ));
    }

    #[test]
    fn rejects_an_empty_candidate_search() {
        let config = CONFIG.replace("n_best = 10", "n_best = 0");
        assert!(matches!(
            DaemonConfig::parse(&config),
            Err(DaemonConfigError::InvalidConversionCandidateCount)
        ));
    }

    #[test]
    fn rejects_an_invalid_personalization_strength() {
        let config = format!(
            "{CONFIG}\n[zenz.personalization]\nbase_ngram = \"/base\"\npersonal_ngram = \"/personal\"\nalpha = -1.0\n"
        );
        assert!(matches!(
            DaemonConfig::parse(&config),
            Err(DaemonConfigError::InvalidPersonalizationAlpha)
        ));
    }
}
