// SPDX-License-Identifier: GPL-3.0-only
use std::path::Path;
use std::time::Duration;

pub async fn file_len(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().map(|m| m.len())
}

/// Returns true when the file size is unchanged across two reads ~150ms apart.
pub async fn file_is_stable(path: &Path) -> bool {
    let Some(first_len) = file_len(path).await else {
        return false;
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    file_len(path).await == Some(first_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_file_is_stable_for_unchanged_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("stable.vpk");
        tokio::fs::write(&path, b"stable-content").await.unwrap();
        assert!(file_is_stable(&path).await);
    }

    #[tokio::test]
    async fn test_file_is_stable_false_while_growing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("growing.vpk");
        let mut file = tokio::fs::File::create(&path).await.unwrap();
        file.write_all(b"start").await.unwrap();

        let path_clone = path.clone();
        let writer = tokio::spawn(async move {
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let mut file = tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(&path_clone)
                    .await
                    .unwrap();
                file.write_all(b"x").await.unwrap();
            }
        });

        // While the file is still growing, stability should eventually read as false.
        let mut saw_unstable = false;
        for _ in 0..5 {
            if !file_is_stable(&path).await {
                saw_unstable = true;
                break;
            }
        }

        writer.abort();
        let _ = writer.await;
        assert!(saw_unstable);
    }
}
