// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::path::PathBuf;
use tracing::{info, warn};
use crate::extractor::traits::Extractor;

pub struct VpkExtractor;

impl VpkExtractor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Extractor for VpkExtractor {
    async fn extract_zip(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("ZIP extraction not supported by VpkExtractor"))
    }
    
    async fn extract_vpk(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()> {
        info!(archive = %archive_path.display(), dest = %dest.display(), "Extracting VPK archive");
        
        tokio::fs::create_dir_all(&dest).await?;
        
        let archive_path_clone = archive_path.clone();
        let dest_clone = dest.clone();
        
        // VPK extraction using sourcepak crate
        // Note: The exact API may vary - this is a placeholder implementation
        // The sourcepak crate API needs to be verified for version 0.3.0
        tokio::task::spawn_blocking(move || {
            // TODO: Implement actual VPK extraction once sourcepak API is verified
            // For now, return an error indicating this needs implementation
            warn!("VPK extraction is not yet implemented - needs sourcepak API verification");
            Err::<(), anyhow::Error>(anyhow::anyhow!(
                "VPK extraction not yet implemented. Archive: {}",
                archive_path_clone.display()
            ))
        })
        .await??;
        
        info!(archive = %archive_path.display(), dest = %dest.display(), "VPK extraction completed");
        Ok(())
    }
}

impl Default for VpkExtractor {
    fn default() -> Self {
        Self::new()
    }
}

