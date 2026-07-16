// SPDX-License-Identifier: GPL-3.0-only
use std::path::PathBuf;

use anyhow::Context;
use tracing::{info, warn};

use super::{
    MapInstallationService, MapOperationFailure, L4d2CenterUpdateAvailable, L4d2CenterUpdateReport,
};
use crate::catalog::l4d2center::{encode_download_url, enrich_with_registry, fetch_index, find_index_entry};
use crate::downloader::traits::Downloader;
use crate::extractor::traits::Extractor;
use crate::registry::models::{MapEntry, SourceKind};
use crate::utils::{md5_matches, verify_file_md5};

/// Returns true when an L4D2Center map should be re-downloaded from the catalog.
pub fn needs_l4d2center_update(
    local_checksum: Option<&str>,
    index_md5: &str,
    force: bool,
) -> bool {
    if force {
        return true;
    }
    match local_checksum {
        Some(local) if md5_matches(local, index_md5) => false,
        Some(_) => true,
        // Missing checksum is unknown — do not treat as outdated unless forced.
        None => force,
    }
}

impl MapInstallationService {
    pub async fn list_l4d2center_catalog(
        &self,
        index_url: &str,
    ) -> anyhow::Result<Vec<crate::catalog::L4d2CenterCatalogEntry>> {
        let entries = fetch_index(index_url).await?;
        enrich_with_registry(entries, self.registry.as_ref()).await
    }

    pub async fn install_l4d2center_by_name(
        &self,
        index_url: &str,
        name: &str,
    ) -> anyhow::Result<MapEntry> {
        let _download_permit = self
            .download_semaphore
            .acquire()
            .await
            .expect("download semaphore closed");

        let entries = fetch_index(index_url).await?;
        let Some(index_entry) = find_index_entry(&entries, name) else {
            anyhow::bail!("Map '{name}' not found in L4D2Center catalog");
        };

        if let Some(existing) = self.find_map_by_installed_path(name).await? {
            if existing
                .checksum
                .as_deref()
                .is_some_and(|checksum| md5_matches(checksum, &index_entry.md5))
            {
                info!(map_id = existing.id, name, "L4D2Center map already up to date");
                return Ok(existing);
            }
        }

        let download_url = encode_download_url(&index_entry.download_link);
        crate::utils::validate_url_resolved(&download_url)
            .await
            .context("Invalid L4D2Center download URL (SSRF protection)")?;

        let downloaded_path = self.zip_downloader.download_zip(&download_url).await?;
        let map_entry = self
            .install_downloaded_file(
                downloaded_path,
                SourceKind::L4d2Center,
                None,
                None,
                Some(download_url),
                Some(name.to_string()),
            )
            .await?;

        let install_path = self.addons_dir.join(&map_entry.installed_path);
        match verify_file_md5(&install_path, &index_entry.md5).await {
            Ok(true) => {}
            Ok(false) => {
                warn!(
                    map_id = map_entry.id,
                    name,
                    "Installed L4D2Center map MD5 does not match catalog entry"
                );
            }
            Err(error) => {
                warn!(
                    map_id = map_entry.id,
                    error = %error,
                    "Failed to verify L4D2Center map MD5 after install"
                );
            }
        }

        Ok(map_entry)
    }

    pub async fn update_l4d2center_maps(
        &self,
        index_url: &str,
        map_id: Option<u64>,
        name: Option<&str>,
        force: bool,
        check_only: bool,
    ) -> anyhow::Result<L4d2CenterUpdateReport> {
        // Do not hold op_lock across downloads — only around disk/registry replace.

        let mut report = L4d2CenterUpdateReport {
            updated: Vec::new(),
            available: Vec::new(),
            skipped: 0,
            failed: Vec::new(),
            not_l4d2center: 0,
        };

        let index_entries = fetch_index(index_url).await?;

        let entries = if let Some(id) = map_id {
            match self.registry.get_map(id).await? {
                Some(entry) => vec![entry],
                None => anyhow::bail!("Map #{id} not found"),
            }
        } else if let Some(catalog_name) = name {
            match self.find_map_by_installed_path(catalog_name).await? {
                Some(entry) => vec![entry],
                None => anyhow::bail!("Map '{catalog_name}' not installed"),
            }
        } else {
            self.registry
                .list_maps()
                .await?
                .into_iter()
                .filter(|entry| entry.source_kind == SourceKind::L4d2Center)
                .collect()
        };

        let targeted = map_id.is_some() || name.is_some();

        for entry in entries {
            if entry.source_kind != SourceKind::L4d2Center {
                if targeted {
                    if let Some(id) = map_id {
                        anyhow::bail!("Map #{id} is not an L4d2Center map");
                    }
                    if let Some(catalog_name) = name {
                        anyhow::bail!("Map '{catalog_name}' is not an L4d2Center map");
                    }
                }
                report.not_l4d2center += 1;
                continue;
            }

            let map_id = entry.id;
            let Some(index_entry) = find_index_entry(&index_entries, &entry.installed_path) else {
                report.failed.push(MapOperationFailure {
                    map_id,
                    error: format!(
                        "Map '{}' no longer listed in L4D2Center catalog",
                        entry.installed_path
                    ),
                });
                continue;
            };

            if !needs_l4d2center_update(entry.checksum.as_deref(), &index_entry.md5, force) {
                report.skipped += 1;
                continue;
            }

            if check_only {
                report.available.push(L4d2CenterUpdateAvailable {
                    name: index_entry.name.clone(),
                    map_id,
                    index_md5: index_entry.md5.clone(),
                    local_md5: entry.checksum.clone(),
                });
                continue;
            }

            info!(map_id, name = %index_entry.name, "Updating outdated L4D2Center map");

            let _download_permit = self
                .download_semaphore
                .acquire()
                .await
                .expect("download semaphore closed");

            let download_url = encode_download_url(&index_entry.download_link);
            let downloaded = match self.zip_downloader.download_zip(&download_url).await {
                Ok(path) => path,
                Err(error) => {
                    report.failed.push(MapOperationFailure {
                        map_id,
                        error: error.to_string(),
                    });
                    continue;
                }
            };

            match self
                .replace_l4d2center_from_download(&entry, downloaded, &index_entry.md5)
                .await
            {
                Ok(updated) => report.updated.push(updated),
                Err(error) => {
                    report.failed.push(MapOperationFailure {
                        map_id,
                        error: error.to_string(),
                    });
                }
            }
        }

        Ok(report)
    }

    pub(super) async fn replace_l4d2center_from_download(
        &self,
        existing: &MapEntry,
        downloaded: PathBuf,
        expected_md5: &str,
    ) -> anyhow::Result<MapEntry> {
        let install_path = self.addons_dir.join(&existing.installed_path);
        crate::utils::validate_path_within_base_new(&install_path, &self.addons_dir)
            .context("Attempted to update map outside of addons directory")?;

        let (source_vpk, temp_cleanup) = self.prepare_vpk_from_download(downloaded).await?;

        let _guard = self.op_lock.lock().await;

        let backup_path = install_path.with_extension("vpk.bak");
        let had_existing = install_path.exists();
        if had_existing {
            tokio::fs::copy(&install_path, &backup_path)
                .await
                .context("Failed to back up existing L4D2Center map before update")?;
        }

        if let Err(error) = crate::utils::atomic_replace_file(&source_vpk, &install_path).await {
            temp_cleanup.cleanup().await;
            let _ = tokio::fs::remove_file(&backup_path).await;
            return Err(error).context("Failed to replace installed L4D2Center map file");
        }
        info!(
            map_id = existing.id,
            dest = %install_path.display(),
            "Replaced installed L4D2Center map file"
        );

        temp_cleanup.cleanup().await;

        let checksum = match crate::utils::calculate_file_md5(&install_path).await {
            Ok(value) => value,
            Err(error) => {
                Self::restore_vpk_backup(&install_path, &backup_path, had_existing).await;
                return Err(error).context("Failed to checksum replaced L4D2Center map");
            }
        };

        if !md5_matches(&checksum, expected_md5) {
            Self::restore_vpk_backup(&install_path, &backup_path, had_existing).await;
            return Err(anyhow::anyhow!(
                "L4D2Center map MD5 mismatch after update (expected {expected_md5}, got {checksum})"
            ));
        }

        let metadata = match self
            .vpk_extractor
            .extract_vpk_metadata(install_path.clone())
            .await
        {
            Ok(metadata) => metadata,
            Err(error) => {
                Self::restore_vpk_backup(&install_path, &backup_path, had_existing).await;
                return Err(error);
            }
        };
        let installed_at = Self::file_modified_time(&install_path)
            .await
            .unwrap_or_else(chrono::Utc::now);

        let mut updated = existing.clone();
        updated.version = Some(metadata.version);
        updated.checksum = Some(checksum);
        updated.checksum_kind = Some("md5".to_string());
        updated.installed_at = installed_at;
        updated.source_kind = SourceKind::L4d2Center;

        if let Err(error) = self.registry.update_map(updated.clone()).await {
            Self::restore_vpk_backup(&install_path, &backup_path, had_existing).await;
            return Err(error).context("Failed to persist L4D2Center map update in registry");
        }

        let _ = tokio::fs::remove_file(&backup_path).await;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_l4d2center_update_respects_force_and_checksum() {
        assert!(needs_l4d2center_update(
            Some("abc"),
            "abc",
            true
        ));
        assert!(!needs_l4d2center_update(
            Some("abc123"),
            "ABC123",
            false
        ));
        assert!(needs_l4d2center_update(
            Some("old"),
            "new",
            false
        ));
        assert!(!needs_l4d2center_update(None, "abc123", false));
        assert!(needs_l4d2center_update(None, "abc123", true));
    }

    #[test]
    fn md5_mismatch_is_detected_by_helper() {
        assert!(!md5_matches(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ));
        assert!(md5_matches("AbC", "abc"));
    }
}
