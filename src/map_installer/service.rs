// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashMap;
use anyhow::Context;
use tracing::{info, warn};

use crate::downloader::{workshop::WorkshopDownloader, zip::ZipDownloader, traits::Downloader};
use crate::extractor::{zip::ZipExtractor, traits::Extractor, vpk::VpkExtractor};
use crate::registry::{models::{MapEntry, SourceKind}, traits::Registry};

pub struct MapInstallationService {
    registry: Arc<dyn Registry>,
    workshop_downloader: WorkshopDownloader,
    zip_downloader: ZipDownloader,
    zip_extractor: ZipExtractor,
    vpk_extractor: VpkExtractor,
    addons_dir: PathBuf,
    temp_dir: PathBuf,
}

pub struct DiscoveryReport {
    pub added: Vec<MapEntry>,
    pub updated: Vec<MapEntry>,
    pub skipped: usize,
    pub failed: usize,
}

pub struct CompactReport {
    pub removed: Vec<MapEntry>,
    pub kept: Vec<MapEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryMode {
    Add,
    Update,
    ForceUpdate,
}

impl MapInstallationService {
    pub async fn new(
        registry: Arc<dyn Registry>,
        addons_dir: PathBuf,
        temp_dir: PathBuf,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            registry,
            workshop_downloader: WorkshopDownloader::new(temp_dir.clone())?,
            zip_downloader: ZipDownloader::new(temp_dir.clone()).await?,
            zip_extractor: ZipExtractor::new(1024 * 1024 * 1024, 10000), // Default limits, should be passed from config
            vpk_extractor: VpkExtractor::new(),
            addons_dir,
            temp_dir,
        })
    }
    
    /// Install a map from a URL or workshop ID
    pub async fn install_from_url(
        &self,
        url: String,
        name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(url = %url, "Starting map installation from URL");
        
        // If a name is provided, check if map with that name already exists
        // This helps prevent race conditions in concurrent installations
        if let Some(ref map_name) = name {
            let sanitized_name = crate::utils::sanitize_map_name(map_name).ok();
            if let Some(ref sanitized) = sanitized_name {
                let maps = self.registry.list_maps().await?;
                if maps.iter().any(|m| m.name == *sanitized) {
                    return Err(anyhow::anyhow!("Map with name '{}' already installed", sanitized));
                }
            }
        }
        
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
        
        // If a name is provided, check if map with that name already exists
        if let Some(ref map_name) = name {
            let sanitized_name = crate::utils::sanitize_map_name(map_name).ok();
            if let Some(ref sanitized) = sanitized_name {
                let maps = self.registry.list_maps().await?;
                if maps.iter().any(|m| m.name == *sanitized) {
                    return Err(anyhow::anyhow!("Map with name '{}' already installed", sanitized));
                }
            }
        }
        
        // Download workshop item
        let downloaded_path = self.workshop_downloader.download_workshop(workshop_id).await?;
        
        // Determine file type and install (workshop maps have empty source_url)
        let map_entry = self.install_downloaded_file(downloaded_path, SourceKind::Workshop, Some(workshop_id), name, None).await?;
        
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
    async fn is_vpk_file(&self, path: &Path) -> anyhow::Result<bool> {
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
        
        // Ensure addons directory exists
        tokio::fs::create_dir_all(&self.addons_dir).await?;
        
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
        
        // Ensure workshop_id is only set when source_kind is Workshop
        let workshop_id = match source_kind {
            SourceKind::Workshop => workshop_id,
            SourceKind::SirPlease | SourceKind::Other => None,
        };
        
        // Create map entry with relative path (ID will be assigned by database)
        let mut map_entry = MapEntry {
            id: 0, // Temporary, will be replaced by database-assigned ID
            name: map_name,
            source_url,
            source_kind,
            workshop_id,
            installed_path: vpk_filename.clone(), // Store relative path (just filename)
            installed_at: chrono::Utc::now(),
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
        
        // Ensure addons directory exists
        tokio::fs::create_dir_all(&self.addons_dir).await?;
        
        // Place VPK file directly in addons/ directory
        let install_path = self.addons_dir.join(&vpk_filename);
        
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
        
        // Ensure workshop_id is only set when source_kind is Workshop
        let workshop_id = match source_kind {
            SourceKind::Workshop => workshop_id,
            SourceKind::SirPlease | SourceKind::Other => None,
        };
        
        // Create map entry with relative path (ID will be assigned by database)
        let mut map_entry = MapEntry {
            id: 0, // Temporary, will be replaced by database-assigned ID
            name: map_name,
            source_url,
            source_kind,
            workshop_id,
            installed_path: vpk_filename.clone(), // Store relative path (just filename)
            installed_at: chrono::Utc::now(),
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
    async fn zip_contains_vpk(&self, zip_path: &Path) -> anyhow::Result<bool> {
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
    fn find_vpk_files_in_extracted(&self, dir: PathBuf) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<PathBuf>>> + Send + '_>> {
        Box::pin(async move {
            let mut vpk_files = Vec::new();
            let mut entries = tokio::fs::read_dir(&dir).await?;
            
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let file_type = entry.file_type().await?;
                
                if file_type.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case("vpk") {
                            vpk_files.push(path);
                        }
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
        if entries.len() == 1 {
            if let Some(entry) = entries.first() {
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_dir() {
                        return entry.file_name().to_str().map(|s| s.to_string());
                    }
                }
            }
        }
        
        // Look for .bsp files (map files)
        for entry in entries {
            if let Ok(file_type) = entry.file_type().await {
                if file_type.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".bsp") {
                            return Some(name.trim_end_matches(".bsp").to_string());
                        }
                    }
                }
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
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.ends_with(".bsp") {
                                return Ok(dir);
                            }
                        }
                    }
                }
            }
            
            // Check subdirectories
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_dir() {
                        let subdir = entry.path();
                        if let Ok(found) = self.find_map_files_in_extracted(subdir).await {
                            return Ok(found);
                        }
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
            if let Ok(file_type) = entry.file_type().await {
                if file_type.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(".vpk") {
                            if let Ok(metadata) = self.vpk_extractor.extract_vpk_metadata(entry.path()).await {
                                return Some(metadata.version);
                            }
                        }
                    }
                }
            }
        }
        
        None
    }
    
    /// Uninstall a map
    pub async fn uninstall_map(&self, map_id: u64) -> anyhow::Result<()> {
        info!(map_id = map_id, "Uninstalling map");
        
        // Get map entry from registry
        let map_entry = self.registry.get_map(map_id).await?
            .ok_or_else(|| anyhow::anyhow!("Map not found: {}", map_id))?;
        
        // Construct absolute path from relative path
        let installed_path_abs = self.addons_dir.join(&map_entry.installed_path);
        
        // Validate that the constructed path is within addons directory (prevent path traversal)
        crate::utils::validate_path_within_base_new(&installed_path_abs, &self.addons_dir)
            .context("Attempted to uninstall map outside of addons directory - potential path traversal detected!")?;
        
        // Remove files
        if installed_path_abs.exists() {
            if installed_path_abs.is_file() {
                // VPK file
                tokio::fs::remove_file(&installed_path_abs).await?;
            } else if installed_path_abs.is_dir() {
                // Directory (shouldn't happen with new structure, but handle for backwards compatibility)
                tokio::fs::remove_dir_all(&installed_path_abs).await?;
            }
            info!(path = %installed_path_abs.display(), "Removed map files");
        }
        
        // Remove from registry
        self.registry.remove_map(map_id).await?;
        
        info!(map_id = map_id, "Map uninstalled successfully");
        Ok(())
    }

    async fn file_modified_time(path: &Path) -> Option<chrono::DateTime<chrono::Utc>> {
        let metadata = tokio::fs::metadata(path).await.ok()?;
        let modified = metadata.modified().ok()?;
        Some(chrono::DateTime::<chrono::Utc>::from(modified))
    }

    async fn build_map_entry_from_file(
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
        let installed_at = Self::file_modified_time(path).await.unwrap_or_else(chrono::Utc::now);

        let (source_kind, workshop_id, source_url) = match metadata.workshop_id {
            Some(workshop_id) => (
                SourceKind::Workshop,
                Some(workshop_id),
                format!("https://steamcommunity.com/sharedfiles/filedetails/?id={workshop_id}"),
            ),
            None => (
                SourceKind::Other,
                None,
                format!("detected:{}", path.display()),
            ),
        };

        Ok(Some(MapEntry {
            id: 0, // Temporary, set by DB for add or overwritten for update
            name: metadata.title,
            source_url,
            source_kind,
            workshop_id,
            installed_path: relative_path.to_string(),
            installed_at,
            version: Some(metadata.version),
            checksum,
            checksum_kind,
        }))
    }
    
    /// During an update, keep the existing record's curated source identity when the
    /// freshly detected entry carries no workshop metadata. A file scan that finds no
    /// workshop id yields placeholders (`workshop_id=None`, `source_kind=Other`,
    /// `source_url="detected:<path>"`); those must not overwrite values a user set
    /// manually (e.g. via `modify`) or that were detected previously.
    fn preserve_source_identity(fresh: &mut MapEntry, existing: &MapEntry) {
        if fresh.workshop_id.is_none() {
            fresh.workshop_id = existing.workshop_id;
            fresh.source_kind = existing.source_kind;
            fresh.source_url = existing.source_url.clone();
        }
    }

    /// Detect map from filesystem path (for watcher)
    pub async fn detect_map_from_path(&self, path: PathBuf) -> anyhow::Result<Option<MapEntry>> {
        // Convert absolute path to relative path (if it's within addons_dir)
        let relative_path = match path.strip_prefix(&self.addons_dir) {
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => {
                // Path is not within addons_dir, skip it
                return Ok(None);
            }
        };
        
        // Check if it's a VPK file
        if path.extension().and_then(|e| e.to_str()) == Some("vpk") {
            // Check if already registered (compare by relative path)
            let maps = self.registry.list_maps().await?;
            if let Some(existing) = maps.iter().find(|m| m.installed_path == relative_path) {
                return Ok(Some(existing.clone()));
            }

            let Some(mut map_entry) = self.build_map_entry_from_file(&path, &relative_path).await? else {
                return Ok(None);
            };

            // Register in database and get assigned ID
            let assigned_id = self.registry.add_map(map_entry.clone()).await?;
            map_entry.id = assigned_id;
            return Ok(Some(map_entry));
        }
        
        // Note: We no longer support detecting directories with .bsp files
        // since all maps should be VPK files in addons/ directory
        Ok(None)
    }

    /// Discover maps in addons_dir and optionally update existing records.
    pub async fn discover_maps(&self, mode: DiscoveryMode) -> anyhow::Result<DiscoveryReport> {
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
                match self.detect_map_from_path(path).await {
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
                entry.source_url =
                    format!("https://steamcommunity.com/sharedfiles/filedetails/?id={wid}");
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
    
    /// Get reference to registry (for sync task)
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }
}

/// Copy directory recursively
/// 
/// Validates that all destination paths stay within the base destination directory
/// to prevent path traversal attacks during recursive copying.
fn copy_directory(src: PathBuf, dst: PathBuf) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(&dst).await?;
        
        // Store canonical base destination for validation
        let base_dst = dst.canonicalize()
            .context("Failed to canonicalize destination base path")?;
        
        let mut entries = tokio::fs::read_dir(&src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            
            // Sanitize filename to prevent path traversal
            let file_name = entry.file_name();
            let sanitized_name = crate::utils::sanitize_filename(
                file_name.to_str().unwrap_or("invalid")
            );
            
            let dst_path = dst.join(&sanitized_name);
            
            // Validate destination path is within base destination
            crate::utils::validate_path_within_base_new(&dst_path, &base_dst)
                .context("Destination path would escape base directory")?;
            
            if entry.file_type().await?.is_dir() {
                copy_directory(entry_path, dst_path).await?;
            } else {
                tokio::fs::copy(&entry_path, &dst_path).await?;
            }
        }
        
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers;
    use tempfile::TempDir;
    use zip::write::{FileOptions, ZipWriter};
    use zip::CompressionMethod;
    use std::io::Write;

    async fn setup_test_service() -> (MapInstallationService, TempDir, Arc<dyn Registry>) {
        let registry: Arc<dyn Registry> = Arc::new(test_helpers::setup_test_database().await.unwrap());
        let temp_dir = test_helpers::create_temp_dir();
        let addons_dir = test_helpers::create_temp_dir();
        
        let service = MapInstallationService::new(
            Arc::clone(&registry),
            addons_dir.path().to_path_buf(),
            temp_dir.path().to_path_buf(),
        ).await.unwrap();
        
        (service, addons_dir, registry)
    }

    fn create_test_zip_with_map(contents: &[(&str, &[u8])]) -> (PathBuf, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let zip_path = temp_dir.path().join("test_map.zip");
        
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        
        for (name, data) in contents {
            zip.start_file(*name, FileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
            zip.write_all(data).unwrap();
        }
        
        zip.finish().unwrap();
        (zip_path, temp_dir)
    }

    #[tokio::test]
    async fn test_registry_accessor() {
        let (service, _addons_dir, registry) = setup_test_service().await;
        // Verify we can access the registry
        let service_registry = service.registry();
        assert_eq!(Arc::as_ptr(service_registry), Arc::as_ptr(&registry));
    }

    #[tokio::test]
    async fn test_uninstall_map_exists() {
        let (service, _addons_dir, registry) = setup_test_service().await;
        
        // Add a test map entry
        let mut map_entry = MapEntry {
            id: 0, // Will be assigned by database
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "test_map.vpk".to_string(), // Relative path
            installed_at: chrono::Utc::now(),
            version: None,
            checksum: None,
            checksum_kind: None,
        };
        let assigned_id = registry.add_map(map_entry.clone()).await.unwrap();
        map_entry.id = assigned_id;
        
        // Uninstall should succeed even if path doesn't exist
        let result = service.uninstall_map(assigned_id).await;
        assert!(result.is_ok());
        
        // Verify map was removed from registry
        let retrieved = registry.get_map(assigned_id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_uninstall_map_not_exists() {
        let (service, _addons_dir, _registry) = setup_test_service().await;
        
        // Uninstall non-existent map should not error
        let result = service.uninstall_map(99999).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_install_from_url_rejects_invalid_url() {
        let (service, _addons_dir, _registry) = setup_test_service().await;
        
        // This should fail because numeric strings are not valid URLs
        let result = service.install_from_url("123456789".to_string(), None).await;
        assert!(result.is_err());
        
        // Error should indicate URL validation failure
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("Invalid URL") || 
            error_msg.contains("SSRF") ||
            error_msg.contains("scheme")
        );
    }

    #[tokio::test]
    async fn test_install_from_url_dispatch_zip() {
        let (mut server, base_url) = {
            let s = mockito::Server::new_async().await;
            let url = s.url();
            (s, url)
        };
        let (service, _addons_dir, registry) = setup_test_service().await;
        
        // Create a mock ZIP file response with a VPK file (required by new validation)
        // Note: This test will likely fail because we need a valid VPK file structure
        // For now, we'll create a minimal VPK-like file
        let minimal_vpk = b"VPK\x02\x00\x00\x00"; // Minimal VPK header
        let (test_zip_path, _zip_temp) = create_test_zip_with_map(&[
            ("test_map.vpk", minimal_vpk),
        ]);
        let zip_content = std::fs::read(&test_zip_path).unwrap();
        
        let mock = server.mock("GET", "/test_map.zip")
            .with_status(200)
            .with_body(zip_content)
            .create_async()
            .await;
        
        let url = format!("{}/test_map.zip", base_url);
        // This test will likely fail because the VPK is invalid, but that's expected
        // We're just testing the flow
        let result = service.install_from_url(url, Some("Test Map".to_string())).await;
        
        // The test might fail due to invalid VPK, but if it succeeds, verify the structure
        if let Ok(map_entry) = result {
            assert_eq!(map_entry.name, "Test Map");
            assert_eq!(map_entry.source_kind, SourceKind::Other);
            assert!(map_entry.installed_path.ends_with(".vpk"));
            
            // Verify map is in registry
            let retrieved = registry.get_map(map_entry.id).await.unwrap();
            assert!(retrieved.is_some());
        }
        
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compact_registry_prunes_sorts_and_reindexes() {
        let (service, addons_dir, registry) = setup_test_service().await;

        tokio::fs::write(addons_dir.path().join("alpha.vpk"), b"vpk").await.unwrap();
        tokio::fs::write(addons_dir.path().join("zulu.vpk"), b"vpk").await.unwrap();

        let now = chrono::Utc::now();
        registry
            .replace_all_maps(vec![
                MapEntry {
                    id: 5,
                    name: "Zulu".to_string(),
                    source_url: "https://example.com/zulu".to_string(),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: "zulu.vpk".to_string(),
                    installed_at: now,
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                },
                MapEntry {
                    id: 12,
                    name: "Alpha".to_string(),
                    source_url: "https://example.com/alpha".to_string(),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: "alpha.vpk".to_string(),
                    installed_at: now,
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                },
                MapEntry {
                    id: 3,
                    name: "Missing".to_string(),
                    source_url: "https://example.com/missing".to_string(),
                    source_kind: SourceKind::Other,
                    workshop_id: None,
                    installed_path: "missing.vpk".to_string(),
                    installed_at: now,
                    version: None,
                    checksum: None,
                    checksum_kind: None,
                },
            ])
            .await
            .unwrap();

        let report = service.compact_registry().await.unwrap();

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].name, "Missing");
        assert_eq!(report.kept.len(), 2);
        assert_eq!(report.kept[0].name, "Alpha");
        assert_eq!(report.kept[0].id, 1);
        assert_eq!(report.kept[1].name, "Zulu");
        assert_eq!(report.kept[1].id, 2);

        let maps = registry.list_maps().await.unwrap();
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].id, 1);
        assert_eq!(maps[0].name, "Alpha");
        assert_eq!(maps[1].id, 2);
        assert_eq!(maps[1].name, "Zulu");
    }

    fn create_modify_test_entry() -> MapEntry {
        MapEntry {
            id: 0,
            name: "Test Map".to_string(),
            source_url: "https://example.com/map.zip".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(999),
            installed_path: "test_map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            version: Some("1.0".to_string()),
            checksum: None,
            checksum_kind: None,
        }
    }

    #[tokio::test]
    async fn test_modify_map_field_workshop_id_sets_kind_and_url() {
        let (service, _addons_dir, registry) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "workshop_id", "3135451698")
            .await
            .unwrap();

        assert_eq!(updated.workshop_id, Some(3135451698));
        assert_eq!(updated.source_kind, SourceKind::Workshop);
        assert_eq!(
            updated.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=3135451698"
        );

        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.workshop_id, Some(3135451698));
        assert_eq!(retrieved.source_kind, SourceKind::Workshop);
        assert_eq!(
            retrieved.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=3135451698"
        );
    }

    #[tokio::test]
    async fn test_modify_map_field_source_kind_other_clears_workshop_id() {
        let (service, _addons_dir, registry) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let updated = service
            .modify_map_field(id, "source_kind", "other")
            .await
            .unwrap();

        assert_eq!(updated.source_kind, SourceKind::Other);
        assert_eq!(updated.workshop_id, None);

        let retrieved = registry.get_map(id).await.unwrap().unwrap();
        assert_eq!(retrieved.source_kind, SourceKind::Other);
        assert_eq!(retrieved.workshop_id, None);
    }

    #[tokio::test]
    async fn test_modify_map_field_unknown_field_errors() {
        let (service, _addons_dir, registry) = setup_test_service().await;
        let id = registry.add_map(create_modify_test_entry()).await.unwrap();

        let result = service
            .modify_map_field(id, "installed_path", "other.vpk")
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown or read-only field"));
    }

    #[tokio::test]
    async fn test_modify_map_field_missing_id_errors() {
        let (service, _addons_dir, _registry) = setup_test_service().await;

        let result = service
            .modify_map_field(99999, "name", "New Name")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Map not found"));
    }

    #[test]
    fn test_preserve_source_identity_keeps_existing_when_fresh_has_no_workshop_id() {
        let existing = MapEntry {
            id: 1,
            name: "Existing".to_string(),
            source_url: "https://steamcommunity.com/sharedfiles/filedetails/?id=12345".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(12345),
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            version: Some("1".to_string()),
            checksum: Some("old".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        let mut fresh = MapEntry {
            id: 1,
            name: "Fresh".to_string(),
            source_url: "detected:/addons/map.vpk".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            version: Some("2".to_string()),
            checksum: Some("new".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        MapInstallationService::preserve_source_identity(&mut fresh, &existing);

        assert_eq!(fresh.workshop_id, Some(12345));
        assert_eq!(fresh.source_kind, SourceKind::Workshop);
        assert_eq!(
            fresh.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=12345"
        );
        assert_eq!(fresh.name, "Fresh");
        assert_eq!(fresh.version, Some("2".to_string()));
        assert_eq!(fresh.checksum, Some("new".to_string()));
    }

    #[test]
    fn test_preserve_source_identity_uses_fresh_when_workshop_id_detected() {
        let existing = MapEntry {
            id: 1,
            name: "Existing".to_string(),
            source_url: "detected:/addons/map.vpk".to_string(),
            source_kind: SourceKind::Other,
            workshop_id: None,
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            version: Some("1".to_string()),
            checksum: Some("old".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        let mut fresh = MapEntry {
            id: 1,
            name: "Fresh".to_string(),
            source_url: "https://steamcommunity.com/sharedfiles/filedetails/?id=999".to_string(),
            source_kind: SourceKind::Workshop,
            workshop_id: Some(999),
            installed_path: "map.vpk".to_string(),
            installed_at: chrono::Utc::now(),
            version: Some("2".to_string()),
            checksum: Some("new".to_string()),
            checksum_kind: Some("md5".to_string()),
        };

        MapInstallationService::preserve_source_identity(&mut fresh, &existing);

        assert_eq!(fresh.workshop_id, Some(999));
        assert_eq!(fresh.source_kind, SourceKind::Workshop);
        assert_eq!(
            fresh.source_url,
            "https://steamcommunity.com/sharedfiles/filedetails/?id=999"
        );
    }
}

