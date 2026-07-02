// SPDX-License-Identifier: GPL-3.0-only
use std::path::PathBuf;

use crate::config::model::Config;

const ALLOWED_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];

impl Config {
    /// Get the addons directory path
    pub fn addons_dir(&self) -> PathBuf {
        self.l4d2_server_dir.join("left4dead2").join("addons")
    }

    /// Validate configuration before starting services.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.sync_interval_secs == 0 {
            anyhow::bail!("sync_interval_secs must be greater than 0");
        }

        if !ALLOWED_LOG_LEVELS.contains(&self.log_level.to_lowercase().as_str()) {
            anyhow::bail!(
                "Invalid log_level '{}', expected one of: {}",
                self.log_level,
                ALLOWED_LOG_LEVELS.join(", ")
            );
        }

        crate::utils::validate_url(&self.l4d2center_index_url)
            .map_err(|e| anyhow::anyhow!("Invalid l4d2center_index_url: {e}"))?;

        if !self.local_api_bind.ip().is_loopback() {
            tracing::warn!(
                addr = %self.local_api_bind,
                "local_api_bind is not loopback; the HTTP API will be reachable on the network without authentication"
            );
        }

        Ok(())
    }
}
