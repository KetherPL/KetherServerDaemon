// SPDX-License-Identifier: GPL-3.0-only
use std::path::Path;

use crate::config::env::{self, keys};
use crate::config::model::Config;

fn load_from_path(path: &Path) -> anyhow::Result<Config> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    } else {
        Ok(Config::default())
    }
}

impl Config {
    /// Load configuration from TOML file with environment variable overrides
    pub fn load() -> anyhow::Result<Self> {
        let config_path =
            std::env::var(keys::CONFIG).unwrap_or_else(|_| "config.toml".to_string());

        let mut config = load_from_path(Path::new(&config_path))?;
        env::apply_env_overrides(&mut config)?;
        Ok(config)
    }
}
