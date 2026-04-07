use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub api_key: String,
    pub host: String,
    pub project_id: Option<i64>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;
        toml::from_str(&content).context("Failed to parse config.toml")
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("posthog")
        .join("config.toml")
}
