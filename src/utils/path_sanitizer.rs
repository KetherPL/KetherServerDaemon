// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Component, Path, PathBuf};
use anyhow::{Context, Result};

/// Sanitize a map name by removing invalid characters and normalizing
///
/// Removes path separators, parent directory references, and other unsafe characters.
/// Normalizes to lowercase and replaces spaces with underscores.
pub fn sanitize_map_name(name: &str) -> Result<String> {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .collect();

    let normalized = sanitized.to_lowercase().trim().replace(' ', "_");

    if normalized.is_empty() {
        return Err(anyhow::anyhow!("Map name cannot be empty after sanitization"));
    }

    if normalized.len() > 255 {
        return Err(anyhow::anyhow!("Map name too long (max 255 characters)"));
    }

    if normalized.starts_with('.') || normalized.starts_with('-') {
        return Err(anyhow::anyhow!("Map name cannot start with '.' or '-'"));
    }

    Ok(normalized)
}

/// Validate that a path is within a base directory
///
/// Prevents directory traversal attacks by ensuring the path doesn't escape
/// the base directory. Uses canonicalization for accurate comparison.
pub fn validate_path_within_base(path: &Path, base: &Path) -> Result<()> {
    let canonical_path = path
        .canonicalize()
        .context("Failed to canonicalize path")?;

    let canonical_base = base
        .canonicalize()
        .context("Failed to canonicalize base path")?;

    if !canonical_path.starts_with(&canonical_base) {
        return Err(anyhow::anyhow!(
            "Path {} is outside base directory {}",
            canonical_path.display(),
            canonical_base.display()
        ));
    }

    Ok(())
}

/// Validate that a path is (or would be) within a base directory.
///
/// Accepts either a path relative to `base`, or an absolute path that must
/// lexically resolve under `base`. Does not require the target to exist.
pub fn validate_path_within_base_new(path: &Path, base: &Path) -> Result<()> {
    let base_norm = if base.exists() {
        base.canonicalize()
            .context("Failed to canonicalize base path")?
    } else {
        normalize_path(base)
    };

    let candidate = if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(&base.join(path))
    };

    if has_parent_dir_component(&candidate) {
        return Err(anyhow::anyhow!(
            "Path contains parent directory reference (..)"
        ));
    }

    if !candidate.starts_with(&base_norm) {
        return Err(anyhow::anyhow!(
            "Path {} is outside base directory {}",
            candidate.display(),
            base_norm.display()
        ));
    }

    // If the path already exists, also enforce symlink-aware containment.
    let existing = if path.exists() {
        Some(path)
    } else if candidate.exists() {
        Some(candidate.as_path())
    } else {
        None
    };

    if let Some(existing_path) = existing {
        validate_path_within_base(existing_path, base)?;
    }

    Ok(())
}

fn has_parent_dir_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

/// Sanitize a filename extracted from a URL or user input
pub fn sanitize_filename(filename: &str) -> String {
    let filename_only = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);

    let sanitized: String = filename_only
        .chars()
        .filter(|c| {
            c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.' || *c == ' '
        })
        .collect();

    sanitized.trim().to_string()
}

/// Normalize a path safely for comparison
///
/// Removes redundant separators and normalizes the path without resolving
/// symlinks or accessing the filesystem.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                normalized.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => {
                normalized.push(part);
            }
        }
    }

    normalized
}

/// Reject archive entry names that could escape the extraction destination.
pub fn validate_archive_entry_name(entry_name: &str) -> Result<()> {
    let trimmed = entry_name.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("Archive entry name is empty"));
    }
    if trimmed.contains('\0') {
        return Err(anyhow::anyhow!("Archive entry name contains NUL"));
    }

    let unified = trimmed.replace('\\', "/");
    if Path::new(&unified).is_absolute()
        || unified.starts_with('/')
        || unified.starts_with('\\')
        || (unified.len() >= 2 && unified.as_bytes()[1] == b':')
    {
        return Err(anyhow::anyhow!(
            "Archive entry {entry_name} is an absolute path"
        ));
    }

    let path = Path::new(&unified);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(anyhow::anyhow!(
            "Archive entry {entry_name} contains unsafe path components"
        ));
    }

    Ok(())
}

/// Ensure `dest.join(entry)` stays under `dest` after lexical normalization.
pub fn resolve_archive_entry_path(dest: &Path, entry_name: &str) -> Result<PathBuf> {
    validate_archive_entry_name(entry_name)?;
    let unified = entry_name.replace('\\', "/");
    let joined = dest.join(unified);
    let dest_norm = normalize_path(dest);
    let out_norm = normalize_path(&joined);
    if !out_norm.starts_with(&dest_norm) {
        return Err(anyhow::anyhow!(
            "Archive entry {entry_name} would escape destination directory"
        ));
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sanitize_map_name_basic() {
        let result = sanitize_map_name("My Test Map").unwrap();
        assert_eq!(result, "my_test_map");
    }

    #[test]
    fn test_sanitize_map_name_with_special_chars() {
        let result = sanitize_map_name("Map!@#$%^&*()Name").unwrap();
        assert_eq!(result, "mapname");
    }

    #[test]
    fn test_sanitize_map_name_with_path_traversal() {
        let result = sanitize_map_name("../../../etc/passwd");
        assert_eq!(result.unwrap(), "etcpasswd");
    }

    #[test]
    fn test_sanitize_map_name_empty() {
        assert!(sanitize_map_name("").is_err());
    }

    #[test]
    fn test_sanitize_map_name_too_long() {
        let long_name = "a".repeat(300);
        assert!(sanitize_map_name(&long_name).is_err());
    }

    #[test]
    fn test_sanitize_filename_basic() {
        assert_eq!(sanitize_filename("test.zip"), "test.zip");
    }

    #[test]
    fn test_sanitize_filename_with_path() {
        assert_eq!(sanitize_filename("/path/to/file.zip"), "file.zip");
    }

    #[test]
    fn test_sanitize_filename_with_unsafe_chars() {
        let result = sanitize_filename("file<script>.zip");
        assert!(!result.contains('<') && !result.contains('>'));
    }

    #[test]
    fn test_validate_path_within_base_valid() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let valid_path = base.join("subdir").join("file.txt");

        std::fs::create_dir_all(valid_path.parent().unwrap()).unwrap();
        std::fs::write(&valid_path, "test").unwrap();

        assert!(validate_path_within_base(&valid_path, base).is_ok());
    }

    #[test]
    fn test_validate_path_within_base_invalid() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let invalid_path = base.parent().unwrap().join("escape.txt");

        std::fs::write(&invalid_path, "test").unwrap();

        assert!(validate_path_within_base(&invalid_path, base).is_err());
    }

    #[test]
    fn test_validate_path_within_base_new_valid() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let new_path = Path::new("subdir/file.txt");

        assert!(validate_path_within_base_new(new_path, base).is_ok());
    }

    #[test]
    fn test_validate_path_within_base_new_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let traversal_path = Path::new("../../../etc/passwd");

        assert!(validate_path_within_base_new(traversal_path, base).is_err());
    }

    #[test]
    fn test_validate_path_within_base_new_rejects_absolute_outside_base() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let absolute = Path::new("/tmp/evil.vpk");

        assert!(validate_path_within_base_new(absolute, base).is_err());
    }

    #[test]
    fn test_validate_path_within_base_new_accepts_absolute_under_base() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let absolute = base.join("map.vpk");

        assert!(validate_path_within_base_new(&absolute, base).is_ok());
    }

    #[test]
    fn test_normalize_path() {
        let path = Path::new("foo/bar/../baz");
        let normalized = normalize_path(path);
        assert_eq!(normalized, PathBuf::from("foo/baz"));
    }

    #[test]
    fn test_validate_archive_entry_name_rejects_absolute_and_traversal() {
        assert!(validate_archive_entry_name("/etc/passwd").is_err());
        assert!(validate_archive_entry_name("C:\\Windows\\evil.vpk").is_err());
        assert!(validate_archive_entry_name("../evil.vpk").is_err());
        assert!(validate_archive_entry_name("nested/ok.vpk").is_ok());
    }

    #[test]
    fn test_resolve_archive_entry_path_stays_under_dest() {
        let dest = Path::new("/addons/extract");
        let resolved = resolve_archive_entry_path(dest, "maps/foo.vpk").unwrap();
        assert_eq!(resolved, PathBuf::from("/addons/extract/maps/foo.vpk"));
        assert!(resolve_archive_entry_path(dest, "../escape.vpk").is_err());
    }
}
