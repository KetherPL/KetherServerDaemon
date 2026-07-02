// SPDX-License-Identifier: GPL-3.0-only
use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Context;
use tracing::{info, warn};

use super::{
    needs_workshop_update, MapInstallationService, MapOperationFailure,
    WorkshopUpdateAvailable, WorkshopUpdateReport,
};
use crate::downloader::steam::{steam_time_to_utc, WorkshopFileDetails};
use crate::extractor::traits::Extractor;
use crate::registry::models::{MapEntry, SourceKind};

impl MapInstallationService {
    /// Re-download outdated Steam Workshop maps and replace installed files in place.
    pub async fn update_workshop_maps(
        &self,
        map_id: Option<u64>,
        force: bool,
        check_only: bool,
    ) -> anyhow::Result<WorkshopUpdateReport> {
        let _guard = self.op_lock.lock().await;

        let mut report = WorkshopUpdateReport {
            updated: Vec::new(),
            available: Vec::new(),
            skipped: 0,
            failed: Vec::new(),
            not_workshop: 0,
        };

        let entries = if let Some(id) = map_id {
            match self.registry.get_map(id).await? {
                Some(entry) => vec![entry],
                None => {
                    return Err(anyhow::anyhow!("Map #{id} not found"));
                }
            }
        } else {
            self.registry.list_maps().await?
        };

        struct Candidate {
            entry: MapEntry,
            workshop_id: u64,
        }

        let mut candidates = Vec::new();
        for entry in entries {
            match self.resolve_workshop_id_for_entry(&entry).await? {
                Some(workshop_id) => candidates.push(Candidate { entry, workshop_id }),
                None => report.not_workshop += 1,
            }
        }

        if candidates.is_empty() {
            return Ok(report);
        }

        let workshop_ids: Vec<u64> = candidates.iter().map(|c| c.workshop_id).collect();
        let details = self
            .workshop_downloader
            .get_workshop_file_details(&workshop_ids)
            .await?;
        let details_by_id: HashMap<u64, WorkshopFileDetails> = details
            .into_iter()
            .map(|detail| (detail.workshop_id, detail))
            .collect();

        for candidate in candidates {
            let map_id = candidate.entry.id;
            let workshop_id = candidate.workshop_id;

            let Some(detail) = details_by_id.get(&workshop_id) else {
                report.failed.push(MapOperationFailure {
                    map_id,
                    error: format!("Workshop item {workshop_id} not found on Steam"),
                });
                continue;
            };

            let steam_updated = steam_time_to_utc(detail.time_updated);
            let install_path = self.addons_dir.join(&candidate.entry.installed_path);
            let local_mtime = Self::file_modified_time(&install_path).await;

            if !needs_workshop_update(
                steam_updated,
                candidate.entry.workshop_updated_at,
                local_mtime,
                force,
            ) {
                report.skipped += 1;
                continue;
            }

            if check_only {
                report.available.push(WorkshopUpdateAvailable {
                    map: candidate.entry,
                    workshop_id,
                    steam_updated_at: steam_updated,
                });
                continue;
            }

            info!(map_id, workshop_id, "Updating outdated workshop map");

            let downloaded = match self
                .workshop_downloader
                .download_from_details(detail)
                .await
            {
                Ok(path) => path,
                Err(error) => {
                    warn!(
                        map_id,
                        workshop_id,
                        error = %error,
                        "Workshop map download failed"
                    );
                    report.failed.push(MapOperationFailure {
                        map_id,
                        error: error.to_string(),
                    });
                    continue;
                }
            };

            match self
                .replace_installed_from_download(&candidate.entry, downloaded, steam_updated)
                .await
            {
                Ok(updated) => report.updated.push(updated),
                Err(error) => {
                    warn!(
                        map_id,
                        workshop_id,
                        error = %error,
                        "Workshop map replace failed"
                    );
                    report.failed.push(MapOperationFailure {
                        map_id,
                        error: error.to_string(),
                    });
                }
            }
        }

        Ok(report)
    }

    async fn resolve_workshop_id_for_entry(
        &self,
        entry: &MapEntry,
    ) -> anyhow::Result<Option<u64>> {
        if let Some(workshop_id) = entry.workshop_id {
            return Ok(Some(workshop_id));
        }

        let path = self.addons_dir.join(&entry.installed_path);
        if !path.exists() {
            return Ok(None);
        }

        let Some(fresh) = self
            .build_map_entry_from_file(&path, &entry.installed_path)
            .await?
        else {
            return Ok(None);
        };

        Ok(fresh.workshop_id)
    }

    async fn replace_installed_from_download(
        &self,
        existing: &MapEntry,
        downloaded: PathBuf,
        workshop_updated_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<MapEntry> {
        let install_path = self.addons_dir.join(&existing.installed_path);
        crate::utils::validate_path_within_base_new(&install_path, &self.addons_dir)
            .context("Attempted to update map outside of addons directory")?;

        let (source_vpk, temp_cleanup) = self.prepare_vpk_from_download(downloaded).await?;

        tokio::fs::copy(&source_vpk, &install_path)
            .await
            .context("Failed to replace installed map file")?;
        info!(
            map_id = existing.id,
            dest = %install_path.display(),
            "Replaced installed workshop map file"
        );

        temp_cleanup.cleanup().await;

        let metadata = self
            .vpk_extractor
            .extract_vpk_metadata(install_path.clone())
            .await?;
        let checksum = crate::utils::calculate_file_md5(&install_path).await.ok();
        let checksum_kind = checksum.as_ref().map(|_| "md5".to_string());
        let installed_at = Self::file_modified_time(&install_path)
            .await
            .unwrap_or_else(chrono::Utc::now);

        let mut updated = existing.clone();
        updated.version = Some(metadata.version);
        updated.checksum = checksum;
        updated.checksum_kind = checksum_kind;
        updated.workshop_updated_at = Some(workshop_updated_at);
        updated.installed_at = installed_at;
        if updated.workshop_id.is_none() {
            updated.workshop_id = metadata.workshop_id;
            if updated.workshop_id.is_some() {
                updated.source_kind = SourceKind::Workshop;
            }
        }

        self.registry.update_map(updated.clone()).await?;
        Ok(updated)
    }

    pub(super) async fn prepare_vpk_from_download(
        &self,
        downloaded: PathBuf,
    ) -> anyhow::Result<(PathBuf, DownloadTempCleanup)> {
        let file_ext = downloaded
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match file_ext.as_str() {
            "vpk" => Ok((downloaded, DownloadTempCleanup::empty())),
            "zip" => {
                if !self.zip_contains_vpk(&downloaded).await? {
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    return Err(anyhow::anyhow!("ZIP file does not contain any .vpk files"));
                }

                let extract_temp = self.temp_dir.join(format!(
                    "update-extract-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                tokio::fs::create_dir_all(&extract_temp).await?;

                if let Err(error) = self
                    .zip_extractor
                    .extract_zip(downloaded.clone(), extract_temp.clone())
                    .await
                {
                    let _ = tokio::fs::remove_dir_all(&extract_temp).await;
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    return Err(error);
                }

                let vpk_files = self.find_vpk_files_in_extracted(extract_temp.clone()).await?;
                if vpk_files.is_empty() {
                    let _ = tokio::fs::remove_dir_all(&extract_temp).await;
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    return Err(anyhow::anyhow!("No .vpk files found in extracted ZIP"));
                }

                Ok((
                    vpk_files[0].clone(),
                    DownloadTempCleanup {
                        paths: vec![downloaded, extract_temp],
                    },
                ))
            }
            "7z" => {
                if !self.sevenz_extractor.sevenz_contains_vpk(&downloaded).await? {
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    return Err(anyhow::anyhow!("7z file does not contain any .vpk files"));
                }

                let extract_temp = self.temp_dir.join(format!(
                    "update-extract-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                tokio::fs::create_dir_all(&extract_temp).await?;

                if let Err(error) = self
                    .sevenz_extractor
                    .extract_sevenz(downloaded.clone(), extract_temp.clone())
                    .await
                {
                    let _ = tokio::fs::remove_dir_all(&extract_temp).await;
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    return Err(error);
                }

                let vpk_files = self.find_vpk_files_in_extracted(extract_temp.clone()).await?;
                if vpk_files.is_empty() {
                    let _ = tokio::fs::remove_dir_all(&extract_temp).await;
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    return Err(anyhow::anyhow!("No .vpk files found in extracted 7z"));
                }

                Ok((
                    vpk_files[0].clone(),
                    DownloadTempCleanup {
                        paths: vec![downloaded, extract_temp],
                    },
                ))
            }
            _ => {
                if self.is_vpk_file(&downloaded).await? {
                    Ok((downloaded, DownloadTempCleanup::empty()))
                } else {
                    let _ = tokio::fs::remove_file(&downloaded).await;
                    Err(anyhow::anyhow!(
                        "Unsupported workshop download file type: {file_ext}"
                    ))
                }
            }
        }
    }
}

pub(super) struct DownloadTempCleanup {
    paths: Vec<PathBuf>,
}

impl DownloadTempCleanup {
    fn empty() -> Self {
        Self { paths: Vec::new() }
    }

    pub(super) async fn cleanup(self) {
        for path in self.paths {
            if path.is_dir() {
                if let Err(error) = tokio::fs::remove_dir_all(&path).await {
                    warn!(path = %path.display(), error = %error, "Failed to clean up temp directory");
                }
            } else if let Err(error) = tokio::fs::remove_file(&path).await {
                warn!(path = %path.display(), error = %error, "Failed to clean up temp file");
            }
        }
    }
}
