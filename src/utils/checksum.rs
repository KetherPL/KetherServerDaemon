// SPDX-License-Identifier: GPL-3.0-only
use anyhow::Context;
use std::path::Path;

/// Compare MD5 hex strings case-insensitively.
pub fn md5_matches(file_md5: &str, expected: &str) -> bool {
    file_md5.eq_ignore_ascii_case(expected)
}

/// Verify a file's MD5 matches the expected hex digest.
pub async fn verify_file_md5(path: &Path, expected: &str) -> anyhow::Result<bool> {
    let actual = calculate_file_md5(path).await?;
    Ok(md5_matches(&actual, expected))
}

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
            let count = reader.read(&mut buffer).with_context(|| {
                format!("Failed to read file for checksum: {}", path.display())
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_is_case_insensitive() {
        assert!(md5_matches("ABC123", "abc123"));
        assert!(!md5_matches("abc123", "abc124"));
    }
}
