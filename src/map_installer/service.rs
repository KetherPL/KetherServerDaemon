// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::downloader::{workshop::WorkshopDownloader, zip::ZipDownloader, traits::Downloader};
use crate::extractor::{zip::ZipExtractor, traits::{Extractor, VpkMetadata}, vpk::VpkExtractor};
use crate::registry::{models::MapEntry, traits::Registry};
use crate::map_installer::url_parser::{parse_url, UrlType};

pub struct MapInstallationService {
    registry: Arc<dyn Registry>,
    workshop_downloader: WorkshopDownloader,
    zip_downloader: ZipDownloader,
    zip_extractor: ZipExtractor,
    vpk_extractor: VpkExtractor,
    addons_dir: PathBuf,
    temp_dir: PathBuf,
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
            zip_extractor: ZipExtractor::new(),
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
        
        let url_type = parse_url(&url)?;
        
        match url_type {
            UrlType::WorkshopId(workshop_id) => {
                self.install_from_workshop_id(workshop_id).await
            }
            UrlType::ZipUrl(zip_url) => {
                self.install_from_zip_url(&zip_url, name).await
            }
        }
    }
    
    /// Install a map from Steam Workshop ID
    pub async fn install_from_workshop_id(
        &self,
        workshop_id: u64,
    ) -> anyhow::Result<MapEntry> {
        info!(workshop_id, "Installing map from Steam Workshop");
        
        // Download workshop item
        let downloaded_path = self.workshop_downloader.download_workshop(workshop_id).await?;
        
        // Determine file type and install
        let map_entry = self.install_downloaded_file(downloaded_path, Some(workshop_id), None).await?;
        
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
        
        // Download ZIP file
        let downloaded_path = self.zip_downloader.download_zip(url).await?;
        
        // Install the downloaded file
        self.install_downloaded_file(downloaded_path, None, name).await
    }
    
    /// Install a downloaded file (ZIP or VPK)
    async fn install_downloaded_file(
        &self,
        file_path: PathBuf,
        workshop_id: Option<u64>,
        provided_name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        let file_ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        
        match file_ext.as_str() {
            "vpk" => self.install_vpk_file(file_path, workshop_id, provided_name).await,
            "zip" => self.install_zip_file(file_path, workshop_id, provided_name).await,
            _ => {
                // Try to infer from content
                if file_path.extension().is_none() || file_ext.is_empty() {
                    // Check if it's a VPK by trying to read it
                    if self.is_vpk_file(&file_path).await? {
                        return self.install_vpk_file(file_path, workshop_id, provided_name).await;
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
        workshop_id: Option<u64>,
        provided_name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(path = %vpk_path.display(), "Installing VPK file");
        
        // Extract metadata to get name and version
        let metadata = self.vpk_extractor.extract_vpk_metadata(vpk_path.clone()).await?;
        
        let map_name = provided_name.unwrap_or_else(|| metadata.title.clone());
        let map_id = Uuid::new_v4().to_string();
        
        // Determine installation path (VPK files go to workshop directory)
        let workshop_dir = self.addons_dir.join("workshop");
        tokio::fs::create_dir_all(&workshop_dir).await?;
        
        let vpk_filename = vpk_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.vpk");
        
        let install_path = workshop_dir.join(vpk_filename);
        
        // Copy VPK file to addons directory
        tokio::fs::copy(&vpk_path, &install_path).await?;
        info!(source = %vpk_path.display(), dest = %install_path.display(), "Copied VPK file");
        
        // Create map entry
        let map_entry = MapEntry {
            id: map_id.clone(),
            name: map_name,
            source_url: format!("workshop:{}", workshop_id.unwrap_or(0)),
            workshop_id,
            installed_path: install_path.clone(),
            installed_at: chrono::Utc::now(),
            version: Some(metadata.version),
        };
        
        // Register in database
        self.registry.add_map(map_entry.clone()).await?;
        
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
        workshop_id: Option<u64>,
        provided_name: Option<String>,
    ) -> anyhow::Result<MapEntry> {
        info!(path = %zip_path.display(), "Installing ZIP file");
        
        // Extract ZIP to temporary directory
        let extract_temp = self.temp_dir.join(format!("extract-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&extract_temp).await?;
        
        self.zip_extractor.extract_zip(zip_path.clone(), extract_temp.clone()).await?;
        
        // Try to detect map name from extracted contents
        let map_name = if let Some(name) = provided_name {
            name
        } else if let Some(name) = self.detect_map_name_from_extracted(&extract_temp).await {
            name
        } else {
            zip_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown_map")
                .to_string()
        };
        
        let map_id = Uuid::new_v4().to_string();
        
        // Determine installation path (extracted maps go to maps directory)
        let maps_dir = self.addons_dir.join("sourcemod").join("maps");
        tokio::fs::create_dir_all(&maps_dir).await?;
        
        let map_install_dir = maps_dir.join(&map_name);
        
        // If directory exists, remove it first (or handle conflict)
        if map_install_dir.exists() {
            warn!(path = %map_install_dir.display(), "Map directory already exists, removing");
            tokio::fs::remove_dir_all(&map_install_dir).await?;
        }
        
        // Move extracted contents to installation directory
        // Find the actual map files (may be nested in subdirectories)
        let source_dir = self.find_map_files_in_extracted(extract_temp.clone()).await?;
        
        // Move the map files to the installation directory
        if source_dir != extract_temp {
            // Copy from nested directory
            copy_directory(source_dir, map_install_dir.clone()).await?;
        } else {
            // Move the entire extracted directory
            tokio::fs::rename(&extract_temp, &map_install_dir).await?;
            // Recreate temp dir for future use
            tokio::fs::create_dir_all(&extract_temp).await?;
        }
        
        // Try to find VPK file in the extracted contents for metadata
        let version = self.find_version_in_extracted(&map_install_dir).await;
        
        // Create map entry
        let source_url = if let Some(ws_id) = workshop_id {
            format!("workshop:{}", ws_id)
        } else {
            format!("zip:{}", zip_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown"))
        };
        
        let map_entry = MapEntry {
            id: map_id.clone(),
            name: map_name.clone(),
            source_url,
            workshop_id,
            installed_path: map_install_dir.clone(),
            installed_at: chrono::Utc::now(),
            version,
        };
        
        // Register in database
        self.registry.add_map(map_entry.clone()).await?;
        
        // Clean up downloaded ZIP file
        if let Err(e) = tokio::fs::remove_file(&zip_path).await {
            warn!(error = %e, path = %zip_path.display(), "Failed to clean up downloaded ZIP");
        }
        
        Ok(map_entry)
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
    pub async fn uninstall_map(&self, map_id: &str) -> anyhow::Result<()> {
        info!(map_id, "Uninstalling map");
        
        // Get map entry from registry
        let map_entry = self.registry.get_map(map_id).await?
            .ok_or_else(|| anyhow::anyhow!("Map not found: {}", map_id))?;
        
        // Remove files
        if map_entry.installed_path.exists() {
            if map_entry.installed_path.is_file() {
                // VPK file
                tokio::fs::remove_file(&map_entry.installed_path).await?;
            } else if map_entry.installed_path.is_dir() {
                // Extracted map directory
                tokio::fs::remove_dir_all(&map_entry.installed_path).await?;
            }
            info!(path = %map_entry.installed_path.display(), "Removed map files");
        }
        
        // Remove from registry
        self.registry.remove_map(map_id).await?;
        
        info!(map_id, "Map uninstalled successfully");
        Ok(())
    }
    
    /// Detect map from filesystem path (for watcher)
    pub async fn detect_map_from_path(&self, path: PathBuf) -> anyhow::Result<Option<MapEntry>> {
        // Check if it's a VPK file
        if path.extension().and_then(|e| e.to_str()) == Some("vpk") {
            if let Ok(metadata) = self.vpk_extractor.extract_vpk_metadata(path.clone()).await {
                // Check if already registered
                let maps = self.registry.list_maps().await?;
                if let Some(existing) = maps.iter().find(|m| m.installed_path == path) {
                    return Ok(Some(existing.clone()));
                }
                
                // Create new entry
                let map_id = Uuid::new_v4().to_string();
                let map_entry = MapEntry {
                    id: map_id.clone(),
                    name: metadata.title,
                    source_url: format!("detected:{}", path.display()),
                    workshop_id: None,
                    installed_path: path,
                    installed_at: chrono::Utc::now(),
                    version: Some(metadata.version),
                };
                
                self.registry.add_map(map_entry.clone()).await?;
                return Ok(Some(map_entry));
            }
        }
        
        // Check if it's a directory with .bsp files
        if path.is_dir() {
            let mut has_bsp = false;
            let mut read_dir = tokio::fs::read_dir(&path).await?;
            while let Some(entry) = read_dir.next_entry().await? {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".bsp") {
                        has_bsp = true;
                        break;
                    }
                }
            }
            
            if has_bsp {
                // Check if already registered
                let maps = self.registry.list_maps().await?;
                if let Some(existing) = maps.iter().find(|m| m.installed_path == path) {
                    return Ok(Some(existing.clone()));
                }
                
                // Create new entry
                let map_id = Uuid::new_v4().to_string();
                let map_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown_map")
                    .to_string();
                
                let map_entry = MapEntry {
                    id: map_id.clone(),
                    name: map_name,
                    source_url: format!("detected:{}", path.display()),
                    workshop_id: None,
                    installed_path: path,
                    installed_at: chrono::Utc::now(),
                    version: None,
                };
                
                self.registry.add_map(map_entry.clone()).await?;
                return Ok(Some(map_entry));
            }
        }
        
        Ok(None)
    }
    
    /// Get reference to registry (for sync task)
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }
}

/// Copy directory recursively
fn copy_directory(src: PathBuf, dst: PathBuf) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(&dst).await?;
        
        let mut entries = tokio::fs::read_dir(&src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let dst_path = dst.join(&file_name);
            
            if entry.file_type().await?.is_dir() {
                copy_directory(entry_path, dst_path).await?;
            } else {
                tokio::fs::copy(&entry_path, &dst_path).await?;
            }
        }
        
        Ok(())
    })
}

