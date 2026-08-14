use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const EXPECTED_CONTEXT_SIZE: usize = 512;
const EXPECTED_BATCH_SIZE: usize = 512;
const EXPECTED_MICRO_BATCH_SIZE: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    pub dictionary: PathBuf,
    pub model: PathBuf,
    pub llama_backend_directory: PathBuf,
    pub runtime_socket: PathBuf,
    pub hunspell: HunspellConfig,
    pub inference: InferenceConfig,
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
            Self::InvalidRuntimeSocket | Self::UnsupportedInferenceProfile(_) => None,
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
dictionary = "/nix/store/dictionary"
model = "/nix/store/model.gguf"
llama_backend_directory = "/nix/store/llama/bin"
runtime_socket = "beankey/daemon.sock"

[hunspell]
english_dictionary = "/nix/store/en_US"
greek_dictionary = "/nix/store/el_GR"

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
}
