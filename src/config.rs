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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_config_home(test: impl FnOnce(&std::path::Path)) {
        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempdir().unwrap();

        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        test(dir.path());
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn config_path_uses_xdg_config_home() {
        with_config_home(|config_home| {
            assert_eq!(
                config_path(),
                config_home.join("posthog").join("config.toml")
            );
        });
    }

    #[test]
    fn load_reads_config_toml() {
        with_config_home(|config_home| {
            let posthog_dir = config_home.join("posthog");
            fs::create_dir_all(&posthog_dir).unwrap();
            fs::write(
                posthog_dir.join("config.toml"),
                r#"
api_key = "phx_test"
host = "https://app.posthog.com"
project_id = 7
"#,
            )
            .unwrap();

            let config = Config::load().unwrap();

            assert_eq!(config.api_key, "phx_test");
            assert_eq!(config.host, "https://app.posthog.com");
            assert_eq!(config.project_id, Some(7));
        });
    }
}
