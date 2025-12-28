// SPDX-License-Identifier: GPL-3.0-only
use anyhow::Context;
use std::path::Path;

/// Calculate MD5 checksum of a file
pub async fn calculate_file_md5(path: &Path) -> anyhow::Result<String> {
    let path = path.to_path_buf();
    
    tokio::task::spawn_blocking(move || {
        use std::fs::File;
        use std::io::{BufReader, Read};
        
        let file = File::open(&path)
            .with_context(|| format!("Failed to open file for checksum: {}", path.display()))?;
        
        let mut reader = BufReader::new(file);
        let mut hasher = md5::Context::new();
        let mut buffer = [0u8; 8192];
        
        loop {
            let count = reader.read(&mut buffer)
                .with_context(|| format!("Failed to read file for checksum: {}", path.display()))?;
            if count == 0 {
                break;
            }
            hasher.consume(&buffer[..count]);
        }
        
        let hash = hasher.compute();
        Ok(format!("{:x}", hash))
    })
    .await
    .context("Checksum calculation task panicked")?
}

