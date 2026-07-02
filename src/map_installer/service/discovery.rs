// SPDX-License-Identifier: GPL-3.0-only
use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Context;
use tracing::{info, warn};

use super::{
    CompactReport, DiscoveryMode, DiscoveryReport, MapInstallationService,
};
use crate::map_installer::helpers::{self, workshop_source_url};
use crate::registry::models::{MapEntry, SourceKind};

impl MapInstallationService {
    /// Sync a map file with the registry: register new VPKs or refresh changed checksums.
    pub async fn sync_map_from_path(&self, path: PathBuf) -> anyhow::Result<Option<MapEntry>> {
        let _guard = self.op_lock.lock().await;

        let relative_path = match helpers::addons_relative_path(&self.addons_dir, &path) {
            Some(rel) => rel,
            None => return Ok(None),
        };

        if path.extension().and_then(|e| e.to_str()) != Some("vpk") {
            return Ok(None);
        }

        if let Some(existing) = self.find_map_by_installed_path(&relative_path).await? {
            let Some(mut fresh_entry) = self.build_map_entry_from_file(&path, &relative_path).await?
            else {
                return Ok(Some(existing));
            };

            if fresh_entry.checksum != existing.checksum {
                fresh_entry.id = existing.id;
                Self::preserve_source_identity(&mut fresh_entry, &existing);
                self.registry.update_map(fresh_entry.clone()).await?;
                return Ok(Some(fresh_entry));
            }

            return Ok(Some(existing));
        }

        self.register_new_map(&path, &relative_path).await
    }

    /// Remove a registry entry when its map file was deleted from disk.
    pub async fn remove_map_by_path(&self, path: PathBuf) -> anyhow::Result<Option<u64>> {
        let _guard = self.op_lock.lock().await;

        let relative_path = match path.strip_prefix(&self.addons_dir) {
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => return Ok(None),
        };

        let abs = self.addons_dir.join(&relative_path);
        if tokio::fs::metadata(&abs).await.is_ok() {
            return Ok(None);
        }

        if let Some(existing) = self.find_map_by_installed_path(&relative_path).await? {
            let id = existing.id;
            self.registry.remove_map(id).await?;
            return Ok(Some(id));
        }

        Ok(None)
    }

    /// Detect map from filesystem path (for watcher)
    pub async fn detect_map_from_path(&self, path: PathBuf) -> anyhow::Result<Option<MapEntry>> {
        let relative_path = match path.strip_prefix(&self.addons_dir) {
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => return Ok(None),
        };

        let _guard = self.op_lock.lock().await;
        self.register_new_map(&path, &relative_path).await
    }

    /// Discover maps in addons_dir and optionally update existing records.
    pub async fn discover_maps(&self, mode: DiscoveryMode) -> anyhow::Result<DiscoveryReport> {
        let _guard = self.op_lock.lock().await;

        let existing_maps = self.registry.list_maps().await?;
        let mut existing_by_path: HashMap<String, MapEntry> = existing_maps
            .into_iter()
            .map(|map| (map.installed_path.clone(), map))
            .collect();

        let vpk_files = self.find_vpk_files_in_extracted(self.addons_dir.clone()).await?;
        let mut report = DiscoveryReport {
            added: Vec::new(),
            updated: Vec::new(),
            skipped: 0,
            failed: 0,
        };

        for path in vpk_files {
            let relative_path = match path.strip_prefix(&self.addons_dir) {
                Ok(relative) => relative.to_string_lossy().to_string(),
                Err(_) => {
                    warn!(path = %path.display(), "Discovery skipped map outside addons directory");
                    report.failed += 1;
                    continue;
                }
            };

            if let Some(existing) = existing_by_path.get(&relative_path).cloned() {
                if mode == DiscoveryMode::Add {
                    report.skipped += 1;
                    continue;
                }

                match self.build_map_entry_from_file(&path, &relative_path).await {
                    Ok(Some(mut fresh_entry)) => {
                        let changed = mode == DiscoveryMode::ForceUpdate
                            || fresh_entry.checksum != existing.checksum;

                        if changed {
                            fresh_entry.id = existing.id;
                            Self::preserve_source_identity(&mut fresh_entry, &existing);
                            match self.registry.update_map(fresh_entry.clone()).await {
                                Ok(()) => {
                                    existing_by_path
                                        .insert(relative_path.clone(), fresh_entry.clone());
                                    report.updated.push(fresh_entry);
                                }
                                Err(error) => {
                                    warn!(error = %error, path = %path.display(), "Discovery failed to update map");
                                    report.failed += 1;
                                }
                            }
                        } else {
                            report.skipped += 1;
                        }
                    }
                    Ok(None) => {
                        report.failed += 1;
                    }
                    Err(error) => {
                        warn!(error = %error, path = %path.display(), "Discovery failed to rebuild map entry");
                        report.failed += 1;
                    }
                }
            } else {
                match self.register_new_map(&path, &relative_path).await {
                    Ok(Some(entry)) => {
                        existing_by_path.insert(entry.installed_path.clone(), entry.clone());
                        report.added.push(entry);
                    }
                    Ok(None) => {
                        report.failed += 1;
                    }
                    Err(error) => {
                        warn!(error = %error, "Discovery failed to register map");
                        report.failed += 1;
                    }
                }
            }
        }

        Ok(report)
    }

    /// Prune registry records whose map files are missing, sort survivors by name,
    /// and reassign sequential IDs starting at 1. Does not delete any files.
    pub async fn compact_registry(&self) -> anyhow::Result<CompactReport> {
        let _guard = self.op_lock.lock().await;

        info!("Compacting map registry");

        let maps = self.registry.list_maps().await?;
        let mut removed = Vec::new();
        let mut survivors = Vec::new();

        for entry in maps {
            let installed_path_abs = self.addons_dir.join(&entry.installed_path);
            let exists = match crate::utils::validate_path_within_base_new(
                &installed_path_abs,
                &self.addons_dir,
            ) {
                Ok(()) => tokio::fs::metadata(&installed_path_abs).await.is_ok(),
                Err(error) => {
                    warn!(
                        map_id = entry.id,
                        path = %entry.installed_path,
                        error = %error,
                        "Compact skipped map with invalid installed path"
                    );
                    false
                }
            };

            if exists {
                survivors.push(entry);
            } else {
                removed.push(entry);
            }
        }

        survivors.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.installed_path.cmp(&b.installed_path))
        });

        for (index, entry) in survivors.iter_mut().enumerate() {
            entry.id = (index + 1) as u64;
        }

        self.registry.replace_all_maps(survivors.clone()).await?;

        info!(
            removed = removed.len(),
            kept = survivors.len(),
            "Registry compact complete"
        );

        Ok(CompactReport {
            removed,
            kept: survivors,
        })
    }

    fn parse_source_kind(value: &str) -> anyhow::Result<SourceKind> {
        match value.to_lowercase().as_str() {
            "workshop" => Ok(SourceKind::Workshop),
            "sirplease" => Ok(SourceKind::SirPlease),
            "l4d2center" => Ok(SourceKind::L4d2Center),
            "other" => Ok(SourceKind::Other),
            other => Err(anyhow::anyhow!(
                "Invalid source_kind '{other}' (expected: workshop, sirplease, other)"
            )),
        }
    }

    /// Modify a single editable field on an existing map record and persist the change.
    pub async fn modify_map_field(
        &self,
        id: u64,
        field: &str,
        value: &str,
    ) -> anyhow::Result<MapEntry> {
        let _guard = self.op_lock.lock().await;

        let mut entry = self
            .registry
            .get_map(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Map not found: {id}"))?;

        match field.to_lowercase().as_str() {
            "name" => entry.name = value.to_string(),
            "source_url" | "url" => entry.source_url = value.to_string(),
            "version" => entry.version = Some(value.to_string()),
            "source_kind" | "kind" => {
                let kind = Self::parse_source_kind(value)?;
                entry.source_kind = kind;
                if kind != SourceKind::Workshop {
                    entry.workshop_id = None;
                }
            }
            "workshop_id" | "wid" => {
                let wid = value
                    .parse::<u64>()
                    .with_context(|| format!("Invalid workshop_id '{value}'"))?;
                entry.workshop_id = Some(wid);
                entry.source_kind = SourceKind::Workshop;
                entry.source_url = workshop_source_url(wid);
            }
            other => {
                return Err(anyhow::anyhow!(
                    "Unknown or read-only field '{other}'. Editable: name, source_url, version, source_kind, workshop_id"
                ));
            }
        }

        self.registry.update_map(entry.clone()).await?;
        Ok(entry)
    }
}
