use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_claude")]
    pub claude: ClaudeConfig,
    #[serde(default = "default_worktree")]
    pub worktree: WorktreeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    #[serde(default = "default_alias")]
    pub alias: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_effort")]
    pub default_effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeConfig {
    #[serde(default = "default_base_dir")]
    pub base_dir: String,
    #[serde(default = "default_houston_path")]
    pub houston_path: String,
    #[serde(default = "default_worktree_defaults")]
    pub defaults: WorktreeDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeDefaults {
    #[serde(default = "default_base_branch")]
    pub base_branch: String,
}

fn default_claude() -> ClaudeConfig {
    ClaudeConfig {
        alias: default_alias(),
        default_model: default_model(),
        default_effort: default_effort(),
    }
}

fn default_worktree() -> WorktreeConfig {
    WorktreeConfig {
        base_dir: default_base_dir(),
        houston_path: default_houston_path(),
        defaults: default_worktree_defaults(),
    }
}

fn default_worktree_defaults() -> WorktreeDefaults {
    WorktreeDefaults {
        base_branch: default_base_branch(),
    }
}

fn default_alias() -> String { "c".to_string() }
fn default_model() -> String { "claude-opus-4-6".to_string() }
fn default_effort() -> String { "high".to_string() }
fn default_base_dir() -> String { "~/dev".to_string() }
fn default_houston_path() -> String { "~/dev/houston".to_string() }
fn default_base_branch() -> String { "origin/master".to_string() }

pub const AVAILABLE_MODELS: &[&str] = &[
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];

pub const AVAILABLE_EFFORTS: &[&str] = &[
    "low",
    "medium",
    "high",
    "max",
    "auto",
];

impl Default for Config {
    fn default() -> Self {
        Config {
            claude: default_claude(),
            worktree: default_worktree(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("ccmux")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config at {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;
        Ok(())
    }

    pub fn exists() -> bool {
        Self::config_path().exists()
    }
}
