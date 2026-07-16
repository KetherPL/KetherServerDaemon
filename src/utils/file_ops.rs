// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

/// Copy `source` onto `dest` via a sibling temp file, fsync, then rename.
pub async fn atomic_replace_file(source: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("target");
    let temp_path = unique_temp_path(parent, file_name);

    tokio::fs::copy(source, &temp_path)
        .await
        .with_context(|| {
            format!(
                "Failed to copy {} to temp {}",
                source.display(),
                temp_path.display()
            )
        })?;

    {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| format!("Failed to open temp file {}", temp_path.display()))?;
        file.sync_all()
            .await
            .with_context(|| format!("Failed to fsync temp file {}", temp_path.display()))?;
        file.flush().await.ok();
    }

    if let Err(error) = tokio::fs::rename(&temp_path, dest).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error).with_context(|| {
            format!(
                "Failed to rename temp {} over {}",
                temp_path.display(),
                dest.display()
            )
        });
    }

    Ok(())
}

fn unique_temp_path(parent: &Path, file_name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".{file_name}.{nanos}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn atomic_replace_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("map.vpk");
        let source = dir.path().join("new.vpk");
        std::fs::write(&dest, b"old").unwrap();
        std::fs::write(&source, b"new-content").unwrap();

        atomic_replace_file(&source, &dest).await.unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new-content");
        assert!(!dir
            .path()
            .read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".tmp")));
    }
}
