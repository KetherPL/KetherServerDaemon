// SPDX-License-Identifier: GPL-3.0-only

//! Disk space utilities for guarding map downloads.
//!
//! Provides a function to check whether sufficient free space is available
//! on the filesystem that hosts a given directory before initiating a download,
//! preventing partial-write failures and potential disk exhaustion.

use std::path::Path;
use anyhow::{Context, Result};

/// VPK file magic bytes: little-endian 0x55AA1234
const VPK_MAGIC: [u8; 4] = [0x34, 0x12, 0xAA, 0x55];

/// Minimum free disk space reserved as a safety buffer (128 MiB).
const SAFETY_BUFFER_BYTES: u64 = 128 * 1024 * 1024;

/// Query the amount of free disk space (in bytes) available on the partition
/// that contains `path`.
///
/// Uses `statvfs` on Linux / macOS via `nix`, falling back to a raw
/// `libc::statvfs` call so we avoid adding a heavyweight crate dependency.
///
/// Returns an error if the query fails (e.g. the path does not exist).
pub fn available_space_bytes(path: &Path) -> Result<u64> {
    use std::mem::MaybeUninit;

    // Obtain a C-compatible path
    let c_path = std::ffi::CString::new(
        path.as_os_str()
            .as_encoded_bytes()
    )
    .context("Path contains an internal NUL byte")?;

    // SAFETY: statvfs is safe when passed a valid, null-terminated path and a
    //         properly aligned stat buffer.  We initialise the struct via MaybeUninit
    //         and only read it after a successful call.
    let stat = unsafe {
        let mut stat: MaybeUninit<libc::statvfs64> = MaybeUninit::uninit();
        let rc = libc::statvfs64(c_path.as_ptr(), stat.as_mut_ptr());
        if rc != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("statvfs64 failed for {}", path.display()));
        }
        stat.assume_init()
    };

    // f_bavail: blocks available to unprivileged processes; f_frsize: fragment size.
    Ok(stat.f_bavail * stat.f_frsize)
}

/// Check that the partition containing `dir` has at least `required_bytes` +
/// [`SAFETY_BUFFER_BYTES`] of free space.
///
/// Returns `Ok(())` if there is enough space, or a descriptive error if not.
pub fn check_sufficient_space(dir: &Path, required_bytes: u64) -> Result<()> {
    let free = available_space_bytes(dir)
        .with_context(|| format!("Failed to query disk space for {}", dir.display()))?;

    let needed = required_bytes.saturating_add(SAFETY_BUFFER_BYTES);
    if free < needed {
        return Err(anyhow::anyhow!(
            "Insufficient disk space: need {needed} bytes ({:.1} MiB) but only {free} bytes \
             ({:.1} MiB) are available on the partition containing '{}'",
            needed as f64 / 1_048_576.0,
            free as f64 / 1_048_576.0,
            dir.display()
        ));
    }

    Ok(())
}

/// Validate the magic bytes of a VPK file to confirm it is a genuine VPK
/// and not a corrupt or truncated download.
///
/// A VPK v1/v2 file always starts with the 4-byte little-endian signature
/// `0x55AA1234` (`[0x34, 0x12, 0xAA, 0x55]`).
///
/// Returns `Ok(())` when the header is valid, or an error describing the
/// problem (wrong magic, too small, or an IO failure).
pub async fn validate_vpk_magic(path: &Path) -> Result<()> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("Failed to open VPK file for integrity check: {}", path.display()))?;

    let mut header = [0u8; 4];
    let n = file
        .read(&mut header)
        .await
        .with_context(|| format!("Failed to read VPK header from {}", path.display()))?;

    if n < 4 {
        return Err(anyhow::anyhow!(
            "VPK file '{}' is too small ({n} bytes) to contain a valid header",
            path.display()
        ));
    }

    if header != VPK_MAGIC {
        return Err(anyhow::anyhow!(
            "VPK file '{}' has invalid magic bytes: expected {:02X?}, got {:02X?}",
            path.display(),
            VPK_MAGIC,
            header
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_space_bytes_for_current_dir() {
        let free = available_space_bytes(Path::new(".")).expect("should query disk space");
        // At a minimum we expect more than 0 bytes free in a live environment.
        assert!(free > 0, "expected non-zero free space, got {free}");
    }

    #[test]
    fn check_sufficient_space_zero_required() {
        // Asking for 0 bytes should always succeed.
        check_sufficient_space(Path::new("."), 0).expect("0 bytes required should always pass");
    }

    #[test]
    fn check_sufficient_space_huge_required_fails() {
        // Asking for u64::MAX bytes should always fail.
        let result = check_sufficient_space(Path::new("."), u64::MAX / 2);
        assert!(result.is_err(), "impossibly large request should fail");
    }
}
