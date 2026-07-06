// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};

use crate::config::env::{self, keys};
use crate::config::model::Config;

pub const CONF_FILE_NAME: &str = "config.toml";

impl Config {
    /// Load configuration from TOML file with environment variable overrides
    pub fn load() -> anyhow::Result<Self> {
        let (config, _) = Self::load_with_path()?;
        Ok(config)
    }

    /// Load configuration and return the resolved config file path for hot reload.
    pub fn load_with_path() -> anyhow::Result<(Self, PathBuf)> {
        let config_path = resolve_config_path();
        let mut config = load_from_path(&config_path)?;
        env::apply_env_overrides(&mut config)?;
        let watched_path = canonicalize_config_path(&config_path);
        Ok((config, watched_path))
    }

    /// Parse configuration from a TOML string (used by hot reload).
    pub fn from_toml_str(content: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(content)?)
    }

    /// Load configuration from a specific path without environment overrides.
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        load_from_path(path)
    }
}

fn resolve_config_path() -> PathBuf {
    std::env::var(keys::CONFIG)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(CONF_FILE_NAME))
}

fn canonicalize_config_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn load_from_path(path: &Path) -> anyhow::Result<Config> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        return Ok(toml::from_str(&contents)?);
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    println!("Creating default config file at: {}", path.display());
    let template = Config::generate_toml_with_comments();
    std::fs::write(path, &template)?;
    Ok(toml::from_str(&template)?)
}
