// SPDX-License-Identifier: GPL-3.0-only
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sevenz_rust::{decompress_file, Password, SevenZReader};
use tracing::info;

use crate::extractor::traits::Extractor;

pub struct SevenZExtractor {
    max_extraction_size: u64,
    max_file_count: u64,
}

impl SevenZExtractor {
    pub fn new(max_extraction_size: u64, max_file_count: u64) -> Self {
        Self {
            max_extraction_size,
            max_file_count,
        }
    }

    fn archive_contains_vpk(archive_path: &Path) -> anyhow::Result<bool> {
        let file = File::open(archive_path)?;
        let len = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(0))?;
        let seven = SevenZReader::new(reader, len, Password::empty())?;
        Ok(seven
            .archive()
            .files
            .iter()
            .any(|entry| entry.name().to_lowercase().ends_with(".vpk")))
    }

    fn validate_archive_limits(archive_path: &Path, max_file_count: u64, max_extraction_size: u64) -> anyhow::Result<()> {
        let file = File::open(archive_path)?;
        let len = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(0))?;
        let seven = SevenZReader::new(reader, len, Password::empty())?;
        let files = &seven.archive().files;
        if files.len() as u64 > max_file_count {
            anyhow::bail!(
                "Archive contains {} files, exceeds maximum of {}",
                files.len(),
                max_file_count
            );
        }

        let total_uncompressed_size: u64 = files.iter().map(|entry| entry.size()).sum();
        if total_uncompressed_size > max_extraction_size {
            anyhow::bail!(
                "Total uncompressed size {} exceeds maximum extraction size {} bytes",
                total_uncompressed_size,
                max_extraction_size
            );
        }

        for entry in files {
            if entry.name().contains("..") {
                anyhow::bail!(
                    "7z entry {} contains parent directory reference",
                    entry.name()
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Extractor for SevenZExtractor {
    async fn extract_zip(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("ZIP extraction not supported by SevenZExtractor"))
    }

    async fn extract_sevenz(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()> {
        info!(
            archive = %archive_path.display(),
            dest = %dest.display(),
            "Extracting 7z archive"
        );

        tokio::fs::create_dir_all(&dest).await?;

        let archive_path_clone = archive_path.clone();
        let dest_clone = dest.clone();
        let max_extraction_size = self.max_extraction_size;
        let max_file_count = self.max_file_count;

        tokio::task::spawn_blocking(move || {
            Self::validate_archive_limits(
                &archive_path_clone,
                max_file_count,
                max_extraction_size,
            )?;
            decompress_file(&archive_path_clone, &dest_clone)
                .map_err(|error| anyhow::anyhow!("7z extraction failed: {error}"))
        })
        .await??;

        info!(
            archive = %archive_path.display(),
            dest = %dest.display(),
            "7z extraction completed"
        );
        Ok(())
    }

    async fn extract_vpk(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("VPK extraction not supported by SevenZExtractor"))
    }

    async fn extract_vpk_metadata(
        &self,
        _archive_path: PathBuf,
    ) -> anyhow::Result<crate::extractor::traits::VpkMetadata> {
        Err(anyhow::anyhow!(
            "VPK metadata extraction not supported by SevenZExtractor"
        ))
    }
}

impl SevenZExtractor {
    pub async fn sevenz_contains_vpk(&self, archive_path: &Path) -> anyhow::Result<bool> {
        let archive_path = archive_path.to_path_buf();
        tokio::task::spawn_blocking(move || Self::archive_contains_vpk(&archive_path)).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sevenz_contains_vpk_rejects_missing_file() {
        let extractor = SevenZExtractor::new(1024, 10);
        let result = extractor
            .sevenz_contains_vpk(Path::new("/nonexistent/archive.7z"))
            .await;
        assert!(result.is_err());
    }
}
