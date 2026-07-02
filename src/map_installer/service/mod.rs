// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::Context;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::map_installer::helpers::{self, workshop_source_url};
use crate::downloader::{
    steam::steam_time_to_utc,
    workshop::WorkshopDownloader,
    zip::ZipDownloader,
    traits::Downloader,
};
use crate::extractor::{zip::ZipExtractor, traits::Extractor, vpk::VpkExtractor};
use crate::registry::{models::{MapEntry, SourceKind}, traits::Registry};
use serde::{Deserialize, Serialize};

pub struct MapInstallationService {
    registry: Arc<dyn Registry>,
    workshop_downloader: WorkshopDownloader,
    zip_downloader: ZipDownloader,
    zip_extractor: ZipExtractor,
    vpk_extractor: VpkExtractor,
    addons_dir: PathBuf,
    temp_dir: PathBuf,
    op_lock: Mutex<()>,
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
            vpk_extractor: VpkExtractor::new(),
            addons_dir,
            temp_dir,
            op_lock: Mutex::new(()),
        })
    }
    
    /// Install a map from a URL or workshop ID
    pub async fn install_from_url(
        &self,
        url: String,
        name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(url = %url, "Starting map installation from URL");
        
        // Validate URL format - should be HTTP/HTTPS
        crate::utils::validate_url(&url)
            .context("Invalid URL format (SSRF protection)")?;
        
        // Install from ZIP URL (url parser no longer needed since workshop_id is separate)
        self.install_from_zip_url(&url, name).await
    }
    
    /// Install a map from Steam Workshop ID
    pub async fn install_from_workshop_id(
        &self,
        workshop_id: u64,
        name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
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
        info!(url = %url, "Installing map from ZIP URL");
        
        // Detect source kind based on URL
        let source_kind = if url.to_lowercase().contains("sirplease.vercel.app") {
            SourceKind::SirPlease
        } else {
            SourceKind::Other
        };
        
        // Download ZIP file
        let downloaded_path = self.zip_downloader.download_zip(url).await?;
        
        // Install the downloaded file
        self.install_downloaded_file(downloaded_path, source_kind, None, name, Some(url.to_string())).await
    }
    
    /// Install a downloaded file (ZIP or VPK)
    async fn install_downloaded_file(
        &self,
        file_path: PathBuf,
        source_kind: SourceKind,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
        source_url: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        let file_ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        
        match file_ext.as_str() {
            "vpk" => self.install_vpk_file(file_path, source_kind, workshop_id, provided_name, source_url).await,
            "zip" => self.install_zip_file(file_path, source_kind, workshop_id, provided_name, source_url).await,
            _ => {
                // Try to infer from content
                if file_path.extension().is_none() || file_ext.is_empty() {
                    // Check if it's a VPK by trying to read it
                    if self.is_vpk_file(&file_path).await? {
                        return self.install_vpk_file(file_path, source_kind, workshop_id, provided_name, source_url).await;
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
    ) -> anyhow::Result<MapEntry> {
        info!(path = %vpk_path.display(), "Installing VPK file");
        
        // Extract metadata to get name and version
        let metadata = self.vpk_extractor.extract_vpk_metadata(vpk_path.clone()).await?;
        
        // Sanitize map name to prevent path traversal
        let raw_map_name = provided_name.unwrap_or_else(|| metadata.title.clone());
        let map_name = crate::utils::sanitize_map_name(&raw_map_name)
            .context("Invalid map name provided")?;
        
        // Sanitize VPK filename to prevent path traversal
        let raw_vpk_filename = vpk_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.vpk");
        let vpk_filename = crate::utils::sanitize_filename(raw_vpk_filename);
        
        // Ensure filename ends with .vpk
        let vpk_filename = if vpk_filename.ends_with(".vpk") {
            vpk_filename
        } else {
            format!("{}.vpk", vpk_filename)
        };
        
        // Place VPK file directly in addons/ directory
        let install_path = self.addons_dir.join(&vpk_filename);

        tokio::fs::create_dir_all(&self.addons_dir).await?;

        let _guard = self.op_lock.lock().await;

        let resolved_workshop_id = match source_kind {
            SourceKind::Workshop => workshop_id,
            SourceKind::SirPlease | SourceKind::Other => None,
        };

        if let Some(wid) = resolved_workshop_id
            && let Some(existing) = self.find_map_by_workshop_id(wid).await? {
                if let Err(e) = tokio::fs::remove_file(&vpk_path).await {
                    warn!(error = %e, path = %vpk_path.display(), "Failed to clean up downloaded file");
                }
                return Ok(existing);
            }

        if self.find_map_by_name(&map_name).await?.is_some() {
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

        // Copy VPK file to addons directory
        tokio::fs::copy(&vpk_path, &install_path).await?;
        info!(source = %vpk_path.display(), dest = %install_path.display(), "Copied VPK file");

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
    ) -> anyhow::Result<MapEntry> {
        info!(path = %zip_path.display(), "Installing ZIP file");
        
        // Validate ZIP contains at least one VPK file before extraction
        if !self.zip_contains_vpk(&zip_path).await? {
            return Err(anyhow::anyhow!("ZIP file does not contain any .vpk files"));
        }
        
        // Extract ZIP to temporary directory
        let extract_temp = self.temp_dir.join(format!("extract-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        tokio::fs::create_dir_all(&extract_temp).await?;
        
        // Extract ZIP with cleanup on error
        if let Err(e) = self.zip_extractor.extract_zip(zip_path.clone(), extract_temp.clone()).await {
            // Clean up temp directory on extraction error
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            return Err(e);
        }
        
        // Find VPK file(s) in extracted contents
        let vpk_files = self.find_vpk_files_in_extracted(extract_temp.clone()).await?;
        if vpk_files.is_empty() {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            return Err(anyhow::anyhow!("No .vpk files found in extracted ZIP"));
        }
        
        // Use the first VPK file (if multiple, we'll install just the first one for now)
        let source_vpk_path = vpk_files[0].clone();
        
        // Extract metadata from VPK to get name and version
        let metadata = self.vpk_extractor.extract_vpk_metadata(source_vpk_path.clone()).await?;
        
        // Determine map name
        let raw_map_name = if let Some(name) = provided_name {
            name
        } else {
            metadata.title.clone()
        };
        let map_name = crate::utils::sanitize_map_name(&raw_map_name)
            .context("Invalid map name detected")?;
        
        // Get VPK filename
        let raw_vpk_filename = source_vpk_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.vpk");
        let vpk_filename = crate::utils::sanitize_filename(raw_vpk_filename);
        
        // Ensure filename ends with .vpk
        let vpk_filename = if vpk_filename.ends_with(".vpk") {
            vpk_filename
        } else {
            format!("{}.vpk", vpk_filename)
        };
        
        // Place VPK file directly in addons/ directory
        let install_path = self.addons_dir.join(&vpk_filename);

        tokio::fs::create_dir_all(&self.addons_dir).await?;

        let _guard = self.op_lock.lock().await;

        let resolved_workshop_id = match source_kind {
            SourceKind::Workshop => workshop_id,
            SourceKind::SirPlease | SourceKind::Other => None,
        };

        if let Some(wid) = resolved_workshop_id
            && let Some(existing) = self.find_map_by_workshop_id(wid).await? {
                let _ = tokio::fs::remove_dir_all(&extract_temp).await;
                if let Err(e) = tokio::fs::remove_file(&zip_path).await {
                    warn!(error = %e, path = %zip_path.display(), "Failed to clean up downloaded ZIP");
                }
                return Ok(existing);
            }

        if self.find_map_by_name(&map_name).await?.is_some() {
            return Err(anyhow::anyhow!(
                "Map with name '{}' already installed",
                map_name
            ));
        }

        if let Some(existing) = self.find_map_by_installed_path(&vpk_filename).await? {
            let _ = tokio::fs::remove_dir_all(&extract_temp).await;
            if let Err(e) = tokio::fs::remove_file(&zip_path).await {
                warn!(error = %e, path = %zip_path.display(), "Failed to clean up downloaded ZIP");
            }
            return Ok(existing);
        }

        // Copy VPK file to addons directory
        tokio::fs::copy(&source_vpk_path, &install_path).await
            .context("Failed to copy VPK file to addons directory")?;

        info!(source = %source_vpk_path.display(), dest = %install_path.display(), "Copied VPK file from ZIP");

        // Calculate MD5 checksum
        let checksum = crate::utils::calculate_file_md5(&install_path).await.ok();
        let checksum_kind = checksum.as_ref().map(|_| "md5".to_string());

        // Determine source URL
        let source_url = source_url.unwrap_or_else(|| {
            zip_path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| format!("zip:{}", n))
                .unwrap_or_else(|| "zip:unknown".to_string())
        });

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
        
        // Clean up downloaded ZIP file and temp directory
        if let Err(e) = tokio::fs::remove_file(&zip_path).await {
            warn!(error = %e, path = %zip_path.display(), "Failed to clean up downloaded ZIP");
        }
        let _ = tokio::fs::remove_dir_all(&extract_temp).await;
        
        Ok(map_entry)
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
            info!(map_id, "Map not found, nothing to uninstall");
            return Ok(());
        };

        // Construct absolute path from relative path
        let installed_path_abs = self.addons_dir.join(&map_entry.installed_path);

        // Validate that the constructed path is within addons directory (prevent path traversal)
        crate::utils::validate_path_within_base_new(&installed_path_abs, &self.addons_dir)
            .context("Attempted to uninstall map outside of addons directory - potential path traversal detected!")?;

        self.registry.remove_map(map_id).await?;

        if installed_path_abs.exists() {
            let removal_result = if installed_path_abs.is_file() {
                tokio::fs::remove_file(&installed_path_abs).await
            } else if installed_path_abs.is_dir() {
                tokio::fs::remove_dir_all(&installed_path_abs).await
            } else {
                Ok(())
            };

            match removal_result {
                Ok(()) => {
                    info!(path = %installed_path_abs.display(), "Removed map files");
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        path = %installed_path_abs.display(),
                        "Registry entry removed but failed to delete map file"
                    );
                }
            }
        }

        info!(map_id = map_id, "Map uninstalled successfully");
        Ok(())
    }

    pub(super) async fn find_map_by_installed_path(
        &self,
        relative_path: &str,
    ) -> anyhow::Result<Option<MapEntry>> {
        let maps = self.registry.list_maps().await?;
        Ok(maps
            .into_iter()
            .find(|m| m.installed_path == relative_path))
    }

    async fn find_map_by_workshop_id(
        &self,
        workshop_id: u64,
    ) -> anyhow::Result<Option<MapEntry>> {
        let maps = self.registry.list_maps().await?;
        Ok(maps
            .into_iter()
            .find(|m| m.workshop_id == Some(workshop_id)))
    }

    async fn find_map_by_name(&self, name: &str) -> anyhow::Result<Option<MapEntry>> {
        let maps = self.registry.list_maps().await?;
        Ok(maps.into_iter().find(|m| m.name == name))
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

    pub(super) async fn build_map_entry_from_file(
        &self,
        path: &Path,
        relative_path: &str,
    ) -> anyhow::Result<Option<MapEntry>> {
        let metadata = match self.vpk_extractor.extract_vpk_metadata(path.to_path_buf()).await {
            Ok(metadata) => metadata,
            Err(_) => return Ok(None),
        };

        let checksum = crate::utils::calculate_file_md5(path).await.ok();
        let checksum_kind = checksum.as_ref().map(|_| "md5".to_string());
        let installed_at = Self::file_modified_time(path)
            .await
            .unwrap_or_else(chrono::Utc::now);

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

    /// Get reference to registry (for sync task)
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }
}


mod discovery;
mod workshop_update;

#[cfg(test)]
mod tests;
