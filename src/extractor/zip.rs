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
    
    async fn extract_vpk_metadata(&self, _archive_path: PathBuf) -> anyhow::Result<crate::extractor::traits::VpkMetadata> {
        Err(anyhow::anyhow!("VPK metadata extraction not supported by ZipExtractor"))
    }
}

impl Default for ZipExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::io::Write;
    use zip::write::{FileOptions, ZipWriter};
    use zip::CompressionMethod;

    fn create_test_zip(contents: &[(&str, &[u8])]) -> (PathBuf, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let zip_path = temp_dir.path().join("test.zip");
        
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
    async fn test_extract_valid_zip() {
        let extractor = ZipExtractor::new();
        let (zip_path, _zip_temp) = create_test_zip(&[
            ("test.txt", b"Hello, World!"),
            ("readme.md", b"# Test Map"),
        ]);
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        extractor.extract_zip(zip_path.clone(), dest_path.clone()).await.unwrap();
        
        // Verify files were extracted
        let test_file = dest_path.join("test.txt");
        let readme_file = dest_path.join("readme.md");
        
        assert!(test_file.exists());
        assert!(readme_file.exists());
        
        let test_content = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(test_content, "Hello, World!");
        
        let readme_content = std::fs::read_to_string(&readme_file).unwrap();
        assert_eq!(readme_content, "# Test Map");
    }

    #[tokio::test]
    async fn test_extract_zip_with_nested_dirs() {
        let extractor = ZipExtractor::new();
        let (zip_path, _zip_temp) = create_test_zip(&[
            ("maps/test_map.bsp", b"BSP data"),
            ("materials/test.vmt", b"VMT data"),
            ("sound/test.wav", b"WAV data"),
        ]);
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        extractor.extract_zip(zip_path.clone(), dest_path.clone()).await.unwrap();
        
        // Verify nested structure
        assert!(dest_path.join("maps/test_map.bsp").exists());
        assert!(dest_path.join("materials/test.vmt").exists());
        assert!(dest_path.join("sound/test.wav").exists());
        
        let bsp_content = std::fs::read(&dest_path.join("maps/test_map.bsp")).unwrap();
        assert_eq!(bsp_content, b"BSP data");
    }

    #[tokio::test]
    async fn test_extract_empty_zip() {
        let extractor = ZipExtractor::new();
        let (zip_path, _zip_temp) = create_test_zip(&[]);
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        // Should not error on empty ZIP
        extractor.extract_zip(zip_path.clone(), dest_path.clone()).await.unwrap();
    }

    #[tokio::test]
    async fn test_extract_zip_nonexistent_file() {
        let extractor = ZipExtractor::new();
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        let nonexistent_zip = PathBuf::from("/nonexistent/path/test.zip");
        
        let result = extractor.extract_zip(nonexistent_zip, dest_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_non_zip_file() {
        let extractor = ZipExtractor::new();
        let temp_dir = TempDir::new().unwrap();
        let fake_zip = temp_dir.path().join("fake.zip");
        std::fs::write(&fake_zip, b"This is not a ZIP file").unwrap();
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        let result = extractor.extract_zip(fake_zip, dest_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_extract_vpk_not_supported() {
        let extractor = ZipExtractor::new();
        let temp_dir = TempDir::new().unwrap();
        let fake_vpk = temp_dir.path().join("test.vpk");
        std::fs::write(&fake_vpk, b"fake vpk").unwrap();
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        let result = extractor.extract_vpk(fake_vpk, dest_path).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("VPK extraction not supported"));
    }

    #[tokio::test]
    async fn test_extract_vpk_metadata_not_supported() {
        let extractor = ZipExtractor::new();
        let temp_dir = TempDir::new().unwrap();
        let fake_vpk = temp_dir.path().join("test.vpk");
        std::fs::write(&fake_vpk, b"fake vpk").unwrap();
        
        let result = extractor.extract_vpk_metadata(fake_vpk).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("VPK metadata extraction not supported"));
    }

    #[tokio::test]
    async fn test_extract_zip_with_special_characters() {
        let extractor = ZipExtractor::new();
        let (zip_path, _zip_temp) = create_test_zip(&[
            ("test file with spaces.txt", b"content with spaces"),
            ("file-with-dashes.txt", b"content with dashes"),
            ("file_with_underscores.txt", b"content with underscores"),
        ]);
        
        let dest_dir = TempDir::new().unwrap();
        let dest_path = dest_dir.path().to_path_buf();
        
        extractor.extract_zip(zip_path.clone(), dest_path.clone()).await.unwrap();
        
        assert!(dest_path.join("test file with spaces.txt").exists());
        assert!(dest_path.join("file-with-dashes.txt").exists());
        assert!(dest_path.join("file_with_underscores.txt").exists());
    }
}

