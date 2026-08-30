#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Auto,
    Logical,
    Visual,
    Passthrough,
}

impl FromStr for Mode {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "logical" => Ok(Self::Logical),
            "visual" => Ok(Self::Visual),
            "passthrough" => Ok(Self::Passthrough),
            _ => Err("mode must be auto, logical, visual, or passthrough"),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Auto => "auto",
            Self::Logical => "logical",
            Self::Visual => "visual",
            Self::Passthrough => "passthrough",
        };
        formatter.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandPolicy {
    pub mode: Mode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    default: CommandPolicy,
    commands: BTreeMap<String, CommandPolicy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    version: u32,
    #[serde(default)]
    default_mode: Mode,
    #[serde(default)]
    commands: BTreeMap<String, PolicyFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    mode: Mode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default: CommandPolicy { mode: Mode::Auto },
            commands: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        Self::load_from_environment(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }

    pub fn load_from_environment(
        xdg_config_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, Box<dyn Error>> {
        let Some(path) = config_path(xdg_config_home, home) else {
            return Ok(Self::default());
        };
        Self::load_from(path)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let path = path.as_ref();
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let parsed: ConfigFile = toml::from_str(&source)?;
        if parsed.version != 1 {
            return Err(
                format!("unsupported config version {}; expected 1", parsed.version).into(),
            );
        }
        let commands = parsed
            .commands
            .into_iter()
            .map(|(name, policy)| (name, CommandPolicy { mode: policy.mode }))
            .collect();
        Ok(Self {
            default: CommandPolicy {
                mode: parsed.default_mode,
            },
            commands,
        })
    }

    pub fn policy_for(&self, command: &OsStr, cli_mode: Option<Mode>) -> CommandPolicy {
        if let Some(mode) = cli_mode {
            return CommandPolicy { mode };
        }
        let name = Path::new(command)
            .file_name()
            .unwrap_or(command)
            .to_string_lossy();
        self.commands
            .get(name.as_ref())
            .copied()
            .unwrap_or(self.default)
    }
}

fn config_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(root) = xdg_config_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Some(root.join("hebrew-tty/config.toml"));
    }
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config/hebrew-tty/config.toml"))
}
