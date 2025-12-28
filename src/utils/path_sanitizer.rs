// SPDX-License-Identifier: GPL-3.0-only
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

/// Sanitize a map name by removing invalid characters and normalizing
/// 
/// Removes path separators, parent directory references, and other unsafe characters.
/// Normalizes to lowercase and replaces spaces with underscores.
pub fn sanitize_map_name(name: &str) -> Result<String> {
    // Remove path separators and unsafe characters
    let sanitized: String = name
        .chars()
        .filter(|c| {
            // Allow alphanumeric, dash, underscore, and spaces
            c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' '
        })
        .collect();
    
    // Normalize: lowercase and replace spaces with underscores
    let normalized = sanitized
        .to_lowercase()
        .trim()
        .replace(' ', "_");
    
    // Validate length
    if normalized.is_empty() {
        return Err(anyhow::anyhow!("Map name cannot be empty after sanitization"));
    }
    
    if normalized.len() > 255 {
        return Err(anyhow::anyhow!("Map name too long (max 255 characters)"));
    }
    
    // Ensure it doesn't start with a dot or dash
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
    // Canonicalize both paths to handle symlinks and .. properly
    let canonical_path = path.canonicalize()
        .context("Failed to canonicalize path")?;
    
    let canonical_base = base.canonicalize()
        .context("Failed to canonicalize base path")?;
    
    // Check if canonical_path starts with canonical_base
    if !canonical_path.starts_with(&canonical_base) {
        return Err(anyhow::anyhow!(
            "Path {} is outside base directory {}",
            canonical_path.display(),
            canonical_base.display()
        ));
    }
    
    Ok(())
}

/// Validate that a path would be within a base directory when created
/// 
/// This version works for paths that don't exist yet by normalizing the path
/// components and checking they don't contain parent directory references.
pub fn validate_path_within_base_new(path: &Path, base: &Path) -> Result<()> {
    // For the new path, resolve it relative to base and check for ..
    let resolved = base.join(path);
    
    // Check if resolved path contains any parent directory references
    // by normalizing it
    let normalized = resolved
        .components()
        .collect::<PathBuf>();
    
    // Check if any component tries to go up
    for component in normalized.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err(anyhow::anyhow!(
                    "Path contains parent directory reference (..)"
                ));
            }
            _ => {}
        }
    }
    
    // If the path exists, also do the canonical check
    if path.exists() {
        validate_path_within_base(path, base)?;
    }
    
    Ok(())
}

/// Sanitize a filename extracted from a URL or user input
/// 
/// Removes path separators and other unsafe characters, ensuring only
/// a valid filename component remains.
pub fn sanitize_filename(filename: &str) -> String {
    // Extract just the filename component (remove any path parts)
    let filename_only = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);
    
    // Remove unsafe characters
    let sanitized: String = filename_only
        .chars()
        .filter(|c| {
            // Allow alphanumeric, dash, underscore, dot, and spaces
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
            std::path::Component::Prefix(_) => {
                normalized.push(component);
            }
            std::path::Component::RootDir => {
                normalized.push(component);
            }
            std::path::Component::CurDir => {
                // Skip . components
            }
            std::path::Component::ParentDir => {
                // Remove the last component if it exists and isn't root
                normalized.pop();
            }
            std::path::Component::Normal(part) => {
                normalized.push(part);
            }
        }
    }
    
    normalized
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
        assert!(result.is_err() || result.unwrap().contains("etc") == false);
    }

    #[test]
    fn test_sanitize_map_name_empty() {
        let result = sanitize_map_name("");
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_map_name_too_long() {
        let long_name = "a".repeat(300);
        let result = sanitize_map_name(&long_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_filename_basic() {
        let result = sanitize_filename("test.zip");
        assert_eq!(result, "test.zip");
    }

    #[test]
    fn test_sanitize_filename_with_path() {
        let result = sanitize_filename("/path/to/file.zip");
        assert_eq!(result, "file.zip");
    }

    #[test]
    fn test_sanitize_filename_with_unsafe_chars() {
        let result = sanitize_filename("file<script>.zip");
        assert!(!result.contains("<") && !result.contains(">"));
    }

    #[test]
    fn test_validate_path_within_base_valid() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        let valid_path = base.join("subdir").join("file.txt");
        
        std::fs::create_dir_all(&valid_path.parent().unwrap()).unwrap();
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
    fn test_normalize_path() {
        let path = Path::new("foo/bar/../baz");
        let normalized = normalize_path(path);
        assert_eq!(normalized, PathBuf::from("foo/baz"));
    }
}

