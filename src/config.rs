use std::path::PathBuf;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub sidebar: SidebarConfig,
    #[serde(default)]
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub detection: DetectionConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub worktree: WorktreeConfig,
    #[serde(default)]
    pub servers: ServersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarConfig {
    pub width: u16,
    pub position: String,
    pub refresh_ms: u64,
    pub sticky: bool,
}
impl Default for SidebarConfig {
    fn default() -> Self { Self { width: 50, position: "left".into(), refresh_ms: 500, sticky: false } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    pub alias: String,
    pub default_model: String,
    pub default_effort: String,
}
impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            alias: "claude".into(),
            default_model: "claude-sonnet-4-6".into(),
            default_effort: "high".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectionConfig {
    pub commands: Vec<String>,
}
impl Default for DetectionConfig {
    fn default() -> Self {
        Self { commands: vec!["claude".into(), "ocli".into(), "ops-cli".into()] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    pub enabled: bool,
    pub macos: bool,
    pub tmux_bell: bool,
    pub repeat_secs: u64,
}
impl Default for NotificationsConfig {
    fn default() -> Self { Self { enabled: true, macos: true, tmux_bell: true, repeat_secs: 0 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeConfig {
    pub base_dir: String,
    pub houston_path: String,
    pub defaults: WorktreeDefaults,
}
impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            base_dir: "~/dev".into(),
            houston_path: "~/dev/houston".into(),
            defaults: WorktreeDefaults::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeDefaults {
    pub base_branch: String,
}
impl Default for WorktreeDefaults {
    fn default() -> Self { Self { base_branch: "origin/master".into() } }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ServersConfig {
    pub extra: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sidebar: SidebarConfig::default(),
            claude: ClaudeConfig::default(),
            detection: DetectionConfig::default(),
            notifications: NotificationsConfig::default(),
            worktree: WorktreeConfig::default(),
            servers: ServersConfig::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("ccmux")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;
        toml::from_str(&text).with_context(|| "Failed to parse config.toml")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)?;
        Ok(())
    }
}

/// (display_name, hex_for_vscode, tmux_colour)
pub const WINDOW_COLORS: &[(&str, &str, &str)] = &[
    ("none",   "",        ""),
    ("red",    "#E06C75", "colour167"),
    ("orange", "#E5C07B", "colour179"),
    ("green",  "#98C379", "colour114"),
    ("blue",   "#61AFEF", "colour75"),
    ("purple", "#C678DD", "colour135"),
    ("pink",   "#FF79C6", "colour212"),
    ("cyan",   "#56B6C2", "colour73"),
];

pub const AVAILABLE_MODELS: &[&str] = &[
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];

pub const AVAILABLE_EFFORTS: &[&str] = &["low", "medium", "high", "max", "auto"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert_eq!(c.sidebar.width, 50);
        assert_eq!(c.claude.default_model, "claude-sonnet-4-6");
        assert_eq!(c.detection.commands, vec!["claude", "ocli", "ops-cli"]);
        assert!(c.notifications.enabled);
    }

    #[test]
    fn round_trip_toml() {
        let c = Config::default();
        let text = toml::to_string_pretty(&c).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.sidebar.width, c.sidebar.width);
        assert_eq!(parsed.claude.alias, c.claude.alias);
    }
}
