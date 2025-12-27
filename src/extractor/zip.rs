// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use tracing::info;
use crate::extractor::traits::Extractor;
use zip::ZipArchive;

pub struct ZipExtractor;

impl ZipExtractor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Extractor for ZipExtractor {
    async fn extract_zip(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()> {
        info!(archive = %archive_path.display(), dest = %dest.display(), "Extracting ZIP archive");
        
        tokio::fs::create_dir_all(&dest).await?;
        
        let archive_path_clone = archive_path.clone();
        let dest_clone = dest.clone();
        
        tokio::task::spawn_blocking(move || {
            let file = File::open(&archive_path_clone)?;
            let mut archive = ZipArchive::new(BufReader::new(file))?;
            
            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let outpath = match file.enclosed_name() {
                    Some(path) => dest_clone.join(path),
                    None => continue,
                };
                
                if file.name().ends_with('/') {
                    std::fs::create_dir_all(&outpath)?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            std::fs::create_dir_all(p)?;
                        }
                    }
                    let mut outfile = File::create(&outpath)?;
                    std::io::copy(&mut file, &mut outfile)?;
                }
            }
            
            Ok::<(), anyhow::Error>(())
        })
        .await??;
        
        info!(archive = %archive_path.display(), dest = %dest.display(), "ZIP extraction completed");
        Ok(())
    }
    
    async fn extract_vpk(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("VPK extraction not supported by ZipExtractor"))
    }
}

impl Default for ZipExtractor {
    fn default() -> Self {
        Self::new()
    }
}

