// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::Context;
use tokio::sync::{Mutex, Semaphore};
use tracing::{info, warn};

use crate::map_installer::helpers::{source_kind_from_url, workshop_source_url};
use crate::map_installer::{ActiveUpdatesState, PendingUpdatesState};
use crate::downloader::{
    steam::steam_time_to_utc,
    workshop::WorkshopDownloader,
    zip::ZipDownloader,
    traits::Downloader,
};
use crate::extractor::{sevenz::SevenZExtractor, zip::ZipExtractor, traits::Extractor, vpk::VpkExtractor};
use crate::registry::{models::{MapEntry, SourceKind}, traits::Registry};
use serde::{Deserialize, Serialize};

pub struct MapInstallationService {
    registry: Arc<dyn Registry>,
    workshop_downloader: WorkshopDownloader,
    zip_downloader: ZipDownloader,
    zip_extractor: ZipExtractor,
    sevenz_extractor: SevenZExtractor,
    vpk_extractor: VpkExtractor,
    addons_dir: PathBuf,
    temp_dir: PathBuf,
    pub(super) op_lock: Mutex<()>,
    /// Caps concurrent heavy download/install work before op_lock is taken.
    pub(super) download_semaphore: Semaphore,
    pub(super) pending_updates: PendingUpdatesState,
    pub(super) active_updates: ActiveUpdatesState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub added: Vec<MapEntry>,
    pub updated: Vec<MapEntry>,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactReport {
    pub removed: Vec<MapEntry>,
    pub kept: Vec<MapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkshopUpdateAvailable {
    pub map: MapEntry,
    pub workshop_id: u64,
    pub steam_updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapOperationFailure {
    pub map_id: u64,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkshopUpdateReport {
    pub updated: Vec<MapEntry>,
    pub available: Vec<WorkshopUpdateAvailable>,
    pub skipped: usize,
    pub failed: Vec<MapOperationFailure>,
    pub not_workshop: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L4d2CenterUpdateAvailable {
    pub name: String,
    pub map_id: u64,
    pub index_md5: String,
    pub local_md5: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L4d2CenterUpdateReport {
    pub updated: Vec<MapEntry>,
    pub available: Vec<L4d2CenterUpdateAvailable>,
    pub skipped: usize,
    pub failed: Vec<MapOperationFailure>,
    pub not_l4d2center: usize,
}

/// Returns true when a workshop map should be re-downloaded from Steam.
pub fn needs_workshop_update(
    steam_time_updated: chrono::DateTime<chrono::Utc>,
    registry_workshop_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    local_file_mtime: Option<chrono::DateTime<chrono::Utc>>,
    force: bool,
) -> bool {
    if force {
        return true;
    }
    if let Some(stored) = registry_workshop_updated_at {
        return steam_time_updated > stored;
    }
    if let Some(mtime) = local_file_mtime {
        return steam_time_updated > mtime;
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMode {
    #[default]
    Add,
    Update,
    ForceUpdate,
}

impl MapInstallationService {
    pub async fn new(
        registry: Arc<dyn Registry>,
        addons_dir: PathBuf,
        temp_dir: PathBuf,
        max_download_size_bytes: u64,
        max_extraction_size_bytes: u64,
        max_extraction_file_count: u64,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            registry,
            workshop_downloader: WorkshopDownloader::new(temp_dir.clone(), max_download_size_bytes)?,
            zip_downloader: ZipDownloader::new(temp_dir.clone(), max_download_size_bytes).await?,
            zip_extractor: ZipExtractor::new(max_extraction_size_bytes, max_extraction_file_count),
            sevenz_extractor: SevenZExtractor::new(max_extraction_size_bytes, max_extraction_file_count),
            vpk_extractor: VpkExtractor::new(),
            addons_dir,
            temp_dir,
            op_lock: Mutex::new(()),
            download_semaphore: Semaphore::new(2),
            pending_updates: PendingUpdatesState::new(),
            active_updates: ActiveUpdatesState::new(),
        })
    }

    pub fn pending_updates(&self) -> PendingUpdatesState {
        self.pending_updates.clone()
    }

    pub fn active_updates(&self) -> ActiveUpdatesState {
        self.active_updates.clone()
    }
    
    /// Install a map from a URL or workshop ID
    pub async fn install_from_url(
        &self,
        url: String,
        name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        let _download_permit = self
            .download_semaphore
            .acquire()
            .await
            .expect("download semaphore closed");
        info!(url = %url, "Starting map installation from URL");

        // Validate URL format - should be HTTP/HTTPS
        crate::utils::validate_url_resolved(&url)
            .await
            .context("Invalid URL format (SSRF protection)")?;

        if let Some(existing) = self.find_map_by_source_url(&url).await? {
            info!(
                map_id = existing.id,
                url = %url,
                "Map with this source URL already installed, skipping download"
            );
            return Ok(existing);
        }

        // Install from ZIP URL (url parser no longer needed since workshop_id is separate)
        self.install_from_zip_url(&url, name).await
    }

    /// Install a map from Steam Workshop ID
    pub async fn install_from_workshop_id(
        &self,
        workshop_id: u64,
        name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        let _download_permit = self
            .download_semaphore
            .acquire()
            .await
            .expect("download semaphore closed");
        info!(workshop_id, "Installing map from Steam Workshop");

        if let Some(existing) = self.find_map_by_workshop_id(workshop_id).await? {
            info!(
                map_id = existing.id,
                workshop_id,
                "Workshop map already installed, skipping download"
            );
            return Ok(existing);
        }

        let details = self
            .workshop_downloader
            .get_workshop_file_details(&[workshop_id])
            .await?;
        let detail = details
            .iter()
            .find(|d| d.workshop_id == workshop_id)
            .ok_or_else(|| anyhow::anyhow!("Workshop item {workshop_id} not found on Steam"))?;

        let downloaded_path = self
            .workshop_downloader
            .download_from_details(detail)
            .await?;
        
        let mut map_entry = self
            .install_downloaded_file(
                downloaded_path,
                SourceKind::Workshop,
                Some(workshop_id),
                name,
                None,
                None,
            )
            .await?;

        map_entry.workshop_updated_at = Some(steam_time_to_utc(detail.time_updated));
        self.registry.update_map(map_entry.clone()).await?;
        
        info!(map_id = %map_entry.id, workshop_id, "Workshop map installed successfully");
        Ok(map_entry)
    }
    
    /// Install a map from ZIP URL
    async fn install_from_zip_url(
        &self,
        url: &str,
        name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(url = %url, "Installing map from URL");

        let source_kind = source_kind_from_url(url);

        let downloaded_path = self.zip_downloader.download_zip(url).await?;

        self.install_downloaded_file(
            downloaded_path,
            source_kind,
            None,
            name,
            Some(url.to_string()),
            None,
        )
        .await
    }
    
    /// Install a downloaded file (ZIP or VPK)
    async fn install_downloaded_file(
        &self,
        file_path: PathBuf,
        source_kind: SourceKind,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
        source_url: Option<String>,
        expected_installed_filename: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        let file_ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match file_ext.as_str() {
            "vpk" => {
                self.install_vpk_file(
                    file_path,
                    source_kind,
                    workshop_id,
                    provided_name,
                    source_url,
                    expected_installed_filename,
                )
                .await
            }
            "zip" => {
                self.install_zip_file(
                    file_path,
                    source_kind,
                    workshop_id,
                    provided_name,
                    source_url,
                    expected_installed_filename,
                )
                .await
            }
            "7z" => {
                self.install_sevenz_file(
                    file_path,
                    source_kind,
                    workshop_id,
                    provided_name,
                    source_url,
                    expected_installed_filename,
                )
                .await
            }
            _ => {
                if file_path.extension().is_none() || file_ext.is_empty() {
                    if self.is_vpk_file(&file_path).await? {
                        return self
                            .install_vpk_file(
                                file_path,
                                source_kind,
                                workshop_id,
                                provided_name,
                                source_url,
                                expected_installed_filename,
                            )
                            .await;
                    }
                }
                Err(anyhow::anyhow!("Unsupported file type: {}", file_ext))
            }
        }
    }
    
    /// Check if a file is a VPK file
    pub(super) async fn is_vpk_file(&self, path: &Path) -> anyhow::Result<bool> {
        // Simple check: try to extract metadata
        match self.vpk_extractor.extract_vpk_metadata(path.to_path_buf()).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
    
    /// Install a VPK file
    async fn install_vpk_file(
        &self,
        vpk_path: PathBuf,
        source_kind: SourceKind,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
        source_url: Option<String>,
        expected_installed_filename: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(path = %vpk_path.display(), "Installing VPK file");
        
        // Extract metadata to get name and version
        let metadata = self.vpk_extractor.extract_vpk_metadata(vpk_path.clone()).await?;
        
        // Sanitize map name to prevent path traversal
        let raw_map_name = provided_name.unwrap_or_else(|| metadata.title.clone());
        let map_name = crate::utils::sanitize_map_name(&raw_map_name)
            .context("Invalid map name provided")?;
        
        let vpk_filename = Self::resolve_vpk_filename(
            expected_installed_filename.as_deref(),
            &vpk_path,
        );
        
        // Place VPK file directly in addons/ directory
        let install_path = self.addons_dir.join(&vpk_filename);

        tokio::fs::create_dir_all(&self.addons_dir).await?;

        let _guard = self.op_lock.lock().await;

        let resolved_workshop_id = match source_kind {
            SourceKind::Workshop => workshop_id,
            SourceKind::SirPlease | SourceKind::L4d2Center | SourceKind::Other => None,
        };

        if let Some(wid) = resolved_workshop_id
            && let Some(existing) = self.find_map_by_workshop_id(wid).await? {
                if let Err(e) = tokio::fs::remove_file(&vpk_path).await {
                    warn!(error = %e, path = %vpk_path.display(), "Failed to clean up downloaded file");
                }
                return Ok(existing);
            }

        if self.find_map_by_name(&map_name).await?.is_some() {
            if let Err(e) = tokio::fs::remove_file(&vpk_path).await {
                warn!(error = %e, path = %vpk_path.display(), "Failed to clean up downloaded file after name collision");
            }
            return Err(anyhow::anyhow!(
                "Map with name '{}' already installed",
                map_name
            ));
        }

        if let Some(existing) = self.find_map_by_installed_path(&vpk_filename).await? {
            if let Err(e) = tokio::fs::remove_file(&vpk_path).await {
                warn!(error = %e, path = %vpk_path.display(), "Failed to clean up downloaded file");
            }
            return Ok(existing);
        }

        // Atomic install into addons directory
        crate::utils::atomic_replace_file(&vpk_path, &install_path)
            .await
            .context("Failed to install VPK file into addons directory")?;
        info!(source = %vpk_path.display(), dest = %install_path.display(), "Installed VPK file");

        // Calculate MD5 checksum
        let checksum = crate::utils::calculate_file_md5(&install_path).await.ok();
        let checksum_kind = checksum.as_ref().map(|_| "md5".to_string());

        // Determine source URL
        let source_url = match source_kind {
            SourceKind::Workshop => "".to_string(), // Empty for workshop maps
            _ => source_url.unwrap_or_else(|| format!("file:{}", vpk_filename)),
        };

        // Create map entry with relative path (ID will be assigned by database)
        let mut map_entry = MapEntry {
            id: 0, // Temporary, will be replaced by database-assigned ID
            name: map_name,
            source_url,
            source_kind,
            workshop_id: resolved_workshop_id,
            installed_path: vpk_filename.clone(), // Store relative path (just filename)
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some(metadata.version),
            checksum,
            checksum_kind,
        };

        // Register in database and get assigned ID
        let assigned_id = match self.registry.add_map(map_entry.clone()).await {
            Ok(id) => id,
            Err(e) => {
                // Clean up installed file on error
                let _ = tokio::fs::remove_file(&install_path).await;
                return Err(e);
            }
        };

        drop(_guard);

        // Update map entry with assigned ID
        map_entry.id = assigned_id;
        
        // Clean up downloaded file
        if let Err(e) = tokio::fs::remove_file(&vpk_path).await {
            warn!(error = %e, path = %vpk_path.display(), "Failed to clean up downloaded file");
        }
        
        Ok(map_entry)
    }
    
    /// Install a ZIP file
    async fn install_zip_file(
        &self,
        zip_path: PathBuf,
        source_kind: SourceKind,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
        source_url: Option<String>,
        expected_installed_filename: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(path = %zip_path.display(), "Installing ZIP file");

        if !self.zip_contains_vpk(&zip_path).await? {
            return Err(anyhow::anyhow!("ZIP file does not contain any .vpk files"));
        }

        let extract_temp = self.temp_dir.join(format!(
            "extract-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&extract_temp).await?;

        if let Err(error) = self
            .zip_extractor
            .extract_zip(zip_path.clone(), extract_temp.clone())
            .await
        {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            return Err(error);
        }

        let fallback_source_url = source_url.clone().unwrap_or_else(|| {
            zip_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| format!("zip:{n}"))
                .unwrap_or_else(|| "zip:unknown".to_string())
        });

        self.install_vpk_from_extracted_dir(
            extract_temp,
            zip_path,
            source_kind,
            workshop_id,
            provided_name,
            Some(fallback_source_url),
            expected_installed_filename,
        )
        .await
    }

    /// Install a 7z file
    async fn install_sevenz_file(
        &self,
        archive_path: PathBuf,
        source_kind: SourceKind,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
        source_url: Option<String>,
        expected_installed_filename: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(path = %archive_path.display(), "Installing 7z file");

        if !self.sevenz_extractor.sevenz_contains_vpk(&archive_path).await? {
            return Err(anyhow::anyhow!("7z file does not contain any .vpk files"));
        }

        let extract_temp = self.temp_dir.join(format!(
            "extract-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&extract_temp).await?;

        if let Err(error) = self
            .sevenz_extractor
            .extract_sevenz(archive_path.clone(), extract_temp.clone())
            .await
        {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            return Err(error);
        }

        let fallback_source_url = source_url.clone().unwrap_or_else(|| {
            archive_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| format!("7z:{n}"))
                .unwrap_or_else(|| "7z:unknown".to_string())
        });

        self.install_vpk_from_extracted_dir(
            extract_temp,
            archive_path,
            source_kind,
            workshop_id,
            provided_name,
            Some(fallback_source_url),
            expected_installed_filename,
        )
        .await
    }

    async fn install_vpk_from_extracted_dir(
        &self,
        extract_temp: PathBuf,
        archive_path: PathBuf,
        source_kind: SourceKind,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
        source_url: Option<String>,
        expected_installed_filename: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        let vpk_files = self.find_vpk_files_in_extracted(extract_temp.clone()).await?;
        if vpk_files.is_empty() {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(anyhow::anyhow!("No .vpk files found in extracted archive"));
        }

        let source_vpk_path = vpk_files[0].clone();
        let metadata = self
            .vpk_extractor
            .extract_vpk_metadata(source_vpk_path.clone())
            .await?;

        let raw_map_name = provided_name.unwrap_or_else(|| metadata.title.clone());
        let map_name = crate::utils::sanitize_map_name(&raw_map_name)
            .context("Invalid map name detected")?;

        let vpk_filename = Self::resolve_vpk_filename(
            expected_installed_filename.as_deref(),
            &source_vpk_path,
        );
        let install_path = self.addons_dir.join(&vpk_filename);

        tokio::fs::create_dir_all(&self.addons_dir).await?;

        let _guard = self.op_lock.lock().await;

        let resolved_workshop_id = match source_kind {
            SourceKind::Workshop => workshop_id,
            SourceKind::SirPlease | SourceKind::L4d2Center | SourceKind::Other => None,
        };

        if let Some(wid) = resolved_workshop_id
            && let Some(existing) = self.find_map_by_workshop_id(wid).await?
        {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Ok(existing);
        }

        if self.find_map_by_name(&map_name).await?.is_some() {
            if let Err(e) = tokio::fs::remove_dir_all(&extract_temp).await {
                warn!(error = %e, path = %extract_temp.display(), "Failed to clean up extract temp after name collision");
            }
            if let Err(e) = tokio::fs::remove_file(&archive_path).await {
                warn!(error = %e, path = %archive_path.display(), "Failed to clean up archive after name collision");
            }
            return Err(anyhow::anyhow!(
                "Map with name '{map_name}' already installed"
            ));
        }

        if let Some(existing) = self.find_map_by_installed_path(&vpk_filename).await? {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Ok(existing);
        }

        crate::utils::atomic_replace_file(&source_vpk_path, &install_path)
            .await
            .context("Failed to install VPK file into addons directory")?;
        info!(
            source = %source_vpk_path.display(),
            dest = %install_path.display(),
            "Installed VPK file from archive"
        );

        let checksum = crate::utils::calculate_file_md5(&install_path).await.ok();
        let checksum_kind = checksum.as_ref().map(|_| "md5".to_string());
        let source_url = source_url.unwrap_or_else(|| {
            archive_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| format!("archive:{n}"))
                .unwrap_or_else(|| "archive:unknown".to_string())
        });

        let mut map_entry = MapEntry {
            id: 0,
            name: map_name,
            source_url,
            source_kind,
            workshop_id: resolved_workshop_id,
            installed_path: vpk_filename.clone(),
            installed_at: chrono::Utc::now(),
            workshop_updated_at: None,
            version: Some(metadata.version),
            checksum,
            checksum_kind,
        };

        let assigned_id = match self.registry.add_map(map_entry.clone()).await {
            Ok(id) => id,
            Err(error) => {
                let _ = tokio::fs::remove_file(&install_path).await;
                return Err(error);
            }
        };

        drop(_guard);
        map_entry.id = assigned_id;

        let _ = tokio::fs::remove_file(&archive_path).await;
        let _ = tokio::fs::remove_dir_all(&extract_temp).await;

        Ok(map_entry)
    }

    fn resolve_vpk_filename(expected: Option<&str>, source_vpk_path: &Path) -> String {
        let raw_vpk_filename = expected
            .map(str::to_string)
            .or_else(|| {
                source_vpk_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "unknown.vpk".to_string());
        let vpk_filename = crate::utils::sanitize_filename(&raw_vpk_filename);
        if vpk_filename.ends_with(".vpk") {
            vpk_filename
        } else {
            format!("{vpk_filename}.vpk")
        }
    }
    
    /// Check if ZIP file contains at least one .vpk file
    pub(super) async fn zip_contains_vpk(&self, zip_path: &Path) -> anyhow::Result<bool> {
        let zip_path = zip_path.to_path_buf();
        
        tokio::task::spawn_blocking(move || {
            use std::fs::File;
            use std::io::BufReader;
            use zip::ZipArchive;
            
            let file = File::open(&zip_path)?;
            let mut archive = ZipArchive::new(BufReader::new(file))?;
            
            for i in 0..archive.len() {
                let file = archive.by_index(i)?;
                let name = file.name();
                if name.to_lowercase().ends_with(".vpk") {
                    return Ok(true);
                }
            }
            
            Ok(false)
        })
        .await?
    }
    
    /// Find all .vpk files in extracted directory (recursive)
    pub(super) fn find_vpk_files_in_extracted(
        &self,
        dir: PathBuf,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<PathBuf>>> + Send + '_>>
    {
        Box::pin(async move {
            let mut vpk_files = Vec::new();
            let mut entries = tokio::fs::read_dir(&dir).await?;
            
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let file_type = entry.file_type().await?;
                
                if file_type.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str())
                        && ext.eq_ignore_ascii_case("vpk") {
                            vpk_files.push(path);
                        }
                } else if file_type.is_dir() {
                    // Recursively search subdirectories
                    let mut sub_files = self.find_vpk_files_in_extracted(path).await?;
                    vpk_files.append(&mut sub_files);
                }
            }
            
            Ok(vpk_files)
        })
    }
    
    /// Detect map name from extracted ZIP contents
    async fn detect_map_name_from_extracted(&self, dir: &Path) -> Option<String> {
        // Look for common map indicators
        let entries = match tokio::fs::read_dir(dir).await {
            Ok(mut entries) => {
                let mut vec = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    vec.push(entry);
                }
                vec
            }
            Err(_) => return None,
        };
        
        // Check if there's a single directory that might be the map
        if entries.len() == 1
            && let Some(entry) = entries.first()
                && let Ok(file_type) = entry.file_type().await
                    && file_type.is_dir() {
                        return entry.file_name().to_str().map(|s| s.to_string());
                    }
        
        // Look for .bsp files (map files)
        for entry in entries {
            if let Ok(file_type) = entry.file_type().await
                && file_type.is_file()
                    && let Some(name) = entry.file_name().to_str()
                        && name.ends_with(".bsp") {
                            return Some(name.trim_end_matches(".bsp").to_string());
                        }
        }
        
        None
    }
    
    /// Find the directory containing map files in extracted ZIP
    fn find_map_files_in_extracted(&self, dir: PathBuf) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<PathBuf>> + Send + '_>> {
        Box::pin(async move {
            // Check if current directory has .bsp files
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await
                    && file_type.is_file()
                        && let Some(name) = entry.file_name().to_str()
                            && name.ends_with(".bsp") {
                                return Ok(dir);
                            }
            }
            
            // Check subdirectories
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await
                    && file_type.is_dir() {
                        let subdir = entry.path();
                        if let Ok(found) = self.find_map_files_in_extracted(subdir).await {
                            return Ok(found);
                        }
                    }
            }
            
            Ok(dir)
        })
    }
    
    /// Find version information in extracted directory
    async fn find_version_in_extracted(&self, dir: &Path) -> Option<String> {
        // Look for VPK files and extract metadata
        let entries = match tokio::fs::read_dir(dir).await.ok()? {
            mut entries => {
                let mut vec = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    vec.push(entry);
                }
                vec
            }
        };
        
        for entry in entries {
            if let Ok(file_type) = entry.file_type().await
                && file_type.is_file()
                    && let Some(name) = entry.file_name().to_str()
                        && name.ends_with(".vpk")
                            && let Ok(metadata) = self.vpk_extractor.extract_vpk_metadata(entry.path()).await {
                                return Some(metadata.version);
                            }
        }
        
        None
    }
    
    /// Uninstall a map
    pub async fn uninstall_map(&self, map_id: u64) -> anyhow::Result<()> {
        let _guard = self.op_lock.lock().await;

        info!(map_id = map_id, "Uninstalling map");

        let Some(map_entry) = self.registry.get_map(map_id).await? else {
            return Err(anyhow::anyhow!("Map #{map_id} not found"));
        };

        // Construct absolute path from relative path
        let installed_path_abs = self.addons_dir.join(&map_entry.installed_path);

        // Validate that the constructed path is within addons directory (prevent path traversal)
        crate::utils::validate_path_within_base_new(&installed_path_abs, &self.addons_dir)
            .context("Attempted to uninstall map outside of addons directory - potential path traversal detected!")?;

        // Delete on disk first so a failed delete does not leave an orphan VPK
        // after the registry entry is already gone.
        if installed_path_abs.exists() {
            let removal_result = if installed_path_abs.is_file() {
                tokio::fs::remove_file(&installed_path_abs).await
            } else if installed_path_abs.is_dir() {
                tokio::fs::remove_dir_all(&installed_path_abs).await
            } else {
                Ok(())
            };

            removal_result.with_context(|| {
                format!(
                    "Failed to delete map files at {}",
                    installed_path_abs.display()
                )
            })?;
            info!(path = %installed_path_abs.display(), "Removed map files");
        }

        self.registry.remove_map(map_id).await?;
        self.pending_updates.remove_map_ids(&[map_id]);
        self.active_updates.clear(map_id);

        info!(map_id = map_id, "Map uninstalled successfully");
        Ok(())
    }

    pub(super) async fn find_map_by_installed_path(
        &self,
        relative_path: &str,
    ) -> anyhow::Result<Option<MapEntry>> {
        self.registry.find_by_installed_path(relative_path).await
    }

    async fn find_map_by_workshop_id(
        &self,
        workshop_id: u64,
    ) -> anyhow::Result<Option<MapEntry>> {
        self.registry.find_by_workshop_id(workshop_id).await
    }

    async fn find_map_by_name(&self, name: &str) -> anyhow::Result<Option<MapEntry>> {
        self.registry.find_by_name(name).await
    }

    async fn find_map_by_source_url(&self, url: &str) -> anyhow::Result<Option<MapEntry>> {
        self.registry.find_by_source_url(url).await
    }

    pub(super) async fn register_new_map(
        &self,
        path: &Path,
        relative_path: &str,
    ) -> anyhow::Result<Option<MapEntry>> {
        if path.extension().and_then(|e| e.to_str()) != Some("vpk") {
            return Ok(None);
        }

        if let Some(existing) = self.find_map_by_installed_path(relative_path).await? {
            return Ok(Some(existing));
        }

        let Some(mut map_entry) = self.build_map_entry_from_file(path, relative_path).await? else {
            return Ok(None);
        };

        let assigned_id = self.registry.add_map(map_entry.clone()).await?;
        map_entry.id = assigned_id;
        Ok(Some(map_entry))
    }

    pub(super) async fn file_modified_time(
        path: &Path,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let metadata = tokio::fs::metadata(path).await.ok()?;
        let modified = metadata.modified().ok()?;
        Some(chrono::DateTime::<chrono::Utc>::from(modified))
    }

    pub(super) async fn restore_vpk_backup(
        install_path: &Path,
        backup_path: &Path,
        had_existing: bool,
    ) {
        if had_existing && backup_path.exists() {
            if let Err(error) = crate::utils::atomic_replace_file(backup_path, install_path).await {
                warn!(
                    path = %install_path.display(),
                    error = %error,
                    "Failed to restore VPK backup after failed update"
                );
            }
        } else if !had_existing {
            let _ = tokio::fs::remove_file(install_path).await;
        }
        let _ = tokio::fs::remove_file(backup_path).await;
    }

    pub(super) async fn build_map_entry_from_file(
        &self,
        path: &Path,
        relative_path: &str,
    ) -> anyhow::Result<Option<MapEntry>> {
        let checksum = crate::utils::calculate_file_md5(path).await.ok();
        let checksum_kind = checksum.as_ref().map(|_| "md5".to_string());
        let installed_at = Self::file_modified_time(path)
            .await
            .unwrap_or_else(chrono::Utc::now);

        let metadata = match self.vpk_extractor.extract_vpk_metadata(path.to_path_buf()).await {
            Ok(metadata) => metadata,
            Err(error) => {
                let fallback_name = Path::new(relative_path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Unknown")
                    .to_string();
                warn!(
                    path = %path.display(),
                    error = %error,
                    fallback_name,
                    "VPK metadata unavailable, using filename fallback"
                );
                return Ok(Some(MapEntry {
                    id: 0,
                    name: fallback_name,
                    source_url: format!("detected:{}", path.display()),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: relative_path.to_string(),
                    installed_at,
                    workshop_updated_at: None,
                    version: None,
                    checksum,
                    checksum_kind,
                }));
            }
        };

        let (source_kind, workshop_id, source_url) = match metadata.workshop_id {
            Some(workshop_id) => (
                SourceKind::Workshop,
                Some(workshop_id),
                workshop_source_url(workshop_id),
            ),
            None => (
                SourceKind::Other,
                None,
                format!("detected:{}", path.display()),
            ),
        };

        Ok(Some(MapEntry {
            id: 0,
            name: metadata.title,
            source_url,
            source_kind,
            workshop_id,
            installed_path: relative_path.to_string(),
            installed_at,
            workshop_updated_at: None,
            version: Some(metadata.version),
            checksum,
            checksum_kind,
        }))
    }

    pub(super) fn preserve_source_identity(fresh: &mut MapEntry, existing: &MapEntry) {
        if fresh.workshop_id.is_none() {
            fresh.workshop_id = existing.workshop_id;
            fresh.source_kind = existing.source_kind;
            fresh.source_url = existing.source_url.clone();
        }
    }

    /// Check the cached Steam transport and evict it when disconnected.
    pub async fn steam_health_check(&self) {
        self.workshop_downloader.health_check().await;
    }

    /// Get reference to registry (for sync task)
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }
}


mod discovery;
mod l4d2center;
mod workshop_update;

#[cfg(test)]
mod tests;
