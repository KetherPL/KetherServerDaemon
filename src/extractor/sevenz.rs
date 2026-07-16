// SPDX-License-Identifier: GPL-3.0-only
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sevenz_rust::{Password, SevenZReader};
use tracing::info;

use crate::extractor::traits::Extractor;
use crate::utils::{resolve_archive_entry_path, validate_archive_entry_name};

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

    /// Validate archive limits/paths and extract in a single archive open.
    fn extract_validated(
        archive_path: &Path,
        dest: &Path,
        max_file_count: u64,
        max_extraction_size: u64,
    ) -> anyhow::Result<()> {
        let file = File::open(archive_path)?;
        let len = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(0))?;
        let mut seven = SevenZReader::new(reader, len, Password::empty())?;

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
            validate_archive_entry_name(entry.name())?;
            resolve_archive_entry_path(dest, entry.name())?;
        }

        let dest = dest.to_path_buf();
        seven
            .for_each_entries(|entry, reader| {
                Self::extract_entry(entry, reader, &dest).map_err(|error| {
                    sevenz_rust::Error::other(error.to_string())
                })
            })
            .map_err(|error| anyhow::anyhow!("7z extraction failed: {error}"))?;

        Ok(())
    }

    fn extract_entry(
        entry: &sevenz_rust::SevenZArchiveEntry,
        reader: &mut dyn Read,
        dest: &Path,
    ) -> anyhow::Result<bool> {
        validate_archive_entry_name(entry.name())?;
        let out_path = resolve_archive_entry_path(dest, entry.name())?;

        if entry.is_directory() {
            std::fs::create_dir_all(&out_path)?;
            return Ok(true);
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = File::create(&out_path)?;
        if entry.size() > 0 {
            let mut writer = BufWriter::new(file);
            std::io::copy(reader, &mut writer)?;
        }

        Ok(true)
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
            Self::extract_validated(
                &archive_path_clone,
                &dest_clone,
                max_file_count,
                max_extraction_size,
            )
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
    use crate::utils::validate_archive_entry_name;

    #[tokio::test]
    async fn sevenz_contains_vpk_rejects_missing_file() {
        let extractor = SevenZExtractor::new(1024, 10);
        let result = extractor
            .sevenz_contains_vpk(Path::new("/nonexistent/archive.7z"))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn rejects_absolute_and_traversal_entry_names() {
        assert!(validate_archive_entry_name("/etc/passwd").is_err());
        assert!(validate_archive_entry_name("C:\\Windows\\evil.vpk").is_err());
        assert!(validate_archive_entry_name("..\\evil.vpk").is_err());
        assert!(validate_archive_entry_name("maps/ok.vpk").is_ok());
    }

    #[test]
    fn extract_entry_rejects_escaping_paths() {
        let dest = Path::new("/tmp/extract-dest");
        let entry = sevenz_rust::SevenZArchiveEntry::from_path(
            Path::new("evil.vpk"),
            "../evil.vpk".to_string(),
        );
        let mut empty: &[u8] = &[];
        let result = SevenZExtractor::extract_entry(&entry, &mut empty, dest);
        assert!(result.is_err());
    }
}
