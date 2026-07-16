// SPDX-License-Identifier: GPL-3.0-only
use async_trait::async_trait;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use tracing::info;
use zip::ZipArchive;

use crate::extractor::limiting_writer::LimitingWriter;
use crate::extractor::traits::Extractor;
use crate::utils::resolve_archive_entry_path;

pub struct ZipExtractor {
    max_extraction_size: u64,
    max_file_count: u64,
}

impl ZipExtractor {
    pub fn new(max_extraction_size: u64, max_file_count: u64) -> Self {
        Self {
            max_extraction_size,
            max_file_count,
        }
    }
}

#[async_trait]
impl Extractor for ZipExtractor {
    async fn extract_zip(&self, archive_path: PathBuf, dest: PathBuf) -> anyhow::Result<()> {
        info!(archive = %archive_path.display(), dest = %dest.display(), "Extracting ZIP archive");

        tokio::fs::create_dir_all(&dest).await?;

        let archive_path_clone = archive_path.clone();
        let dest_clone = dest.clone();
        let max_extraction_size = self.max_extraction_size;
        let max_file_count = self.max_file_count;

        tokio::task::spawn_blocking(move || {
            let file = File::open(&archive_path_clone)?;
            let mut archive = ZipArchive::new(BufReader::new(file))?;

            if archive.len() as u64 > max_file_count {
                return Err(anyhow::anyhow!(
                    "Archive contains {} files, exceeds maximum of {} files",
                    archive.len(),
                    max_file_count
                ));
            }

            let mut total_written: u64 = 0;

            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let raw_name = file.name().to_string();
                let entry_name = file.enclosed_name().ok_or_else(|| {
                    anyhow::anyhow!("ZIP entry has unsafe or absolute path: {raw_name}")
                })?;
                let entry_name_str = entry_name.to_string_lossy().into_owned();
                let outpath = resolve_archive_entry_path(&dest_clone, &entry_name_str)?;

                if raw_name.ends_with('/') {
                    std::fs::create_dir_all(&outpath)?;
                    continue;
                }

                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let remaining = max_extraction_size.saturating_sub(total_written);
                if remaining == 0 {
                    return Err(anyhow::anyhow!(
                        "Total extraction size exceeds maximum of {} bytes",
                        max_extraction_size
                    ));
                }

                let outfile = File::create(&outpath)?;
                let mut limited = LimitingWriter::new(outfile, remaining);
                match std::io::copy(&mut file, &mut limited) {
                    Ok(_) => {
                        limited.flush()?;
                        total_written = total_written.saturating_add(limited.written());
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(&outpath);
                        return Err(anyhow::anyhow!(
                            "ZIP extraction failed for {entry_name_str}: {error}"
                        ));
                    }
                }

                if total_written > max_extraction_size {
                    return Err(anyhow::anyhow!(
                        "Total extraction size {} exceeds maximum {} bytes",
                        total_written,
                        max_extraction_size
                    ));
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

    async fn extract_sevenz(&self, _archive_path: PathBuf, _dest: PathBuf) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("7z extraction not supported by ZipExtractor"))
    }

    async fn extract_vpk_metadata(
        &self,
        _archive_path: PathBuf,
    ) -> anyhow::Result<crate::extractor::traits::VpkMetadata> {
        Err(anyhow::anyhow!(
            "VPK metadata extraction not supported by ZipExtractor"
        ))
    }
}

impl Default for ZipExtractor {
    fn default() -> Self {
        Self::new(1024 * 1024 * 1024, 10000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::{FileOptions, ZipWriter};
    use zip::CompressionMethod;

    fn create_test_zip(contents: &[(&str, &[u8])]) -> (PathBuf, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let zip_path = temp_dir.path().join("test.zip");

        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);

        for (name, data) in contents {
            zip.start_file(
                *name,
                FileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(data).unwrap();
        }

        zip.finish().unwrap();
        (zip_path, temp_dir)
    }

    #[tokio::test]
    async fn test_extract_valid_zip() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let (zip_path, _zip_temp) = create_test_zip(&[
            ("test.txt", b"Hello, World!"),
            ("readme.md", b"# Test Map"),
        ]);

        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();

        extractor
            .extract_zip(zip_path.clone(), dest_path.clone())
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(dest_path.join("test.txt")).unwrap(),
            "Hello, World!"
        );
        assert_eq!(
            std::fs::read_to_string(dest_path.join("readme.md")).unwrap(),
            "# Test Map"
        );
    }

    #[tokio::test]
    async fn test_extract_zip_with_nested_dirs() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let (zip_path, _zip_temp) = create_test_zip(&[
            ("maps/test_map.bsp", b"BSP data"),
            ("materials/test.vmt", b"VMT data"),
            ("sound/test.wav", b"WAV data"),
        ]);

        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();

        extractor
            .extract_zip(zip_path.clone(), dest_path.clone())
            .await
            .unwrap();

        assert!(dest_path.join("maps/test_map.bsp").exists());
        assert!(dest_path.join("materials/test.vmt").exists());
        assert!(dest_path.join("sound/test.wav").exists());
    }

    #[tokio::test]
    async fn test_extract_empty_zip() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let (zip_path, _zip_temp) = create_test_zip(&[]);
        let dest_dir = TempDir::new().unwrap();
        extractor
            .extract_zip(zip_path, dest_dir.path().to_path_buf())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_extract_zip_nonexistent_file() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let dest_dir = TempDir::new().unwrap();
        let result = extractor
            .extract_zip(
                PathBuf::from("/nonexistent/path/test.zip"),
                dest_dir.path().to_path_buf(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_non_zip_file() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let temp_dir = TempDir::new().unwrap();
        let fake_zip = temp_dir.path().join("fake.zip");
        std::fs::write(&fake_zip, b"This is not a ZIP file").unwrap();
        let dest_dir = TempDir::new().unwrap();
        assert!(extractor
            .extract_zip(fake_zip, dest_dir.path().to_path_buf())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_extract_rejects_path_traversal_entry() {
        let extractor = ZipExtractor::new(1024 * 1024, 100);
        let temp_dir = TempDir::new().unwrap();
        let zip_path = temp_dir.path().join("evil.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = ZipWriter::new(file);
            // enclosed_name() rejects .. so this should be skipped/err depending on zip crate
            zip.start_file(
                "../escape.txt",
                FileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(b"evil").unwrap();
            zip.finish().unwrap();
        }
        let dest_dir = TempDir::new().unwrap();
        let result = extractor
            .extract_zip(zip_path, dest_dir.path().to_path_buf())
            .await;
        // Either errors or skips unsafe name; must not write outside dest.
        assert!(!temp_dir.path().join("escape.txt").exists());
        if result.is_ok() {
            assert!(!dest_dir.path().join("escape.txt").exists());
        }
    }

    #[tokio::test]
    async fn test_extract_aborts_when_actual_bytes_exceed_limit() {
        let extractor = ZipExtractor::new(8, 100);
        let (zip_path, _zip_temp) = create_test_zip(&[("big.txt", b"0123456789abcdef")]);
        let dest_dir = TempDir::new().unwrap();
        let result = extractor
            .extract_zip(zip_path, dest_dir.path().to_path_buf())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceed"));
    }

    #[tokio::test]
    async fn test_extract_vpk_not_supported() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let temp_dir = TempDir::new().unwrap();
        let fake_vpk = temp_dir.path().join("test.vpk");
        std::fs::write(&fake_vpk, b"fake vpk").unwrap();
        let dest_dir = TempDir::new().unwrap();
        let result = extractor
            .extract_vpk(fake_vpk, dest_dir.path().to_path_buf())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_vpk_metadata_not_supported() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let temp_dir = TempDir::new().unwrap();
        let fake_vpk = temp_dir.path().join("test.vpk");
        std::fs::write(&fake_vpk, b"fake vpk").unwrap();
        assert!(extractor.extract_vpk_metadata(fake_vpk).await.is_err());
    }

    #[tokio::test]
    async fn test_extract_zip_with_special_characters() {
        let extractor = ZipExtractor::new(1024 * 1024 * 1024, 10000);
        let (zip_path, _zip_temp) = create_test_zip(&[
            ("test file with spaces.txt", b"content with spaces"),
            ("file-with-dashes.txt", b"content with dashes"),
            ("file_with_underscores.txt", b"content with underscores"),
        ]);
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        extractor
            .extract_zip(zip_path, dest_path.clone())
            .await
            .unwrap();
        assert!(dest_path.join("test file with spaces.txt").exists());
        assert!(dest_path.join("file-with-dashes.txt").exists());
        assert!(dest_path.join("file_with_underscores.txt").exists());
    }
}
