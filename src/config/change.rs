// SPDX-License-Identifier: GPL-3.0-only
use crate::config::model::Config;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConfigChange {
    pub live_applied: Vec<&'static str>,
    pub requires_restart: Vec<&'static str>,
    pub unchanged: bool,
}

impl ConfigChange {
    pub fn log(&self) {
        if self.unchanged {
            return;
        }

        if !self.live_applied.is_empty() {
            println!(
                "Config hot reload: applied live fields: {}",
                self.live_applied.join(", ")
            );
        }

        if !self.requires_restart.is_empty() {
            eprintln!(
                "Config hot reload: restart required for fields: {}",
                self.requires_restart.join(", ")
            );
        }
    }
}

impl Config {
    pub fn diff(&self, new: &Config) -> ConfigChange {
        let mut change = ConfigChange::default();

        if self.hidden_workshop_ids != new.hidden_workshop_ids {
            change.live_applied.push("hidden_workshop_ids");
        }
        if self.hidden_map_ids != new.hidden_map_ids {
            change.live_applied.push("hidden_map_ids");
        }
        if self.sync_interval_secs != new.sync_interval_secs {
            change.live_applied.push("sync_interval_secs");
        }
        if self.l4d2center_index_url != new.l4d2center_index_url {
            change.live_applied.push("l4d2center_index_url");
        }
        if self.backend_api_url != new.backend_api_url {
            change.live_applied.push("backend_api_url");
        }
        if self.backend_api_key != new.backend_api_key {
            change.live_applied.push("backend_api_key");
        }
        if self.local_api_key != new.local_api_key {
            change.live_applied.push("local_api_key");
        }

        if self.l4d2_server_dir != new.l4d2_server_dir {
            change.requires_restart.push("l4d2_server_dir");
        }
        if self.registry_path != new.registry_path {
            change.requires_restart.push("registry_path");
        }
        if self.local_api_bind != new.local_api_bind {
            change.requires_restart.push("local_api_bind");
        }
        if self.max_download_size_bytes != new.max_download_size_bytes {
            change.requires_restart.push("max_download_size_bytes");
        }
        if self.max_extraction_size_bytes != new.max_extraction_size_bytes {
            change.requires_restart.push("max_extraction_size_bytes");
        }
        if self.max_extraction_file_count != new.max_extraction_file_count {
            change.requires_restart.push("max_extraction_file_count");
        }
        if self.log_level != new.log_level {
            change.requires_restart.push("log_level");
        }

        change.unchanged = change.live_applied.is_empty() && change.requires_restart.is_empty();
        change
    }
}
