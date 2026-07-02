// SPDX-License-Identifier: GPL-3.0-only
use std::path::Path;

use crate::config::env::{self, keys};
use crate::config::model::Config;

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
