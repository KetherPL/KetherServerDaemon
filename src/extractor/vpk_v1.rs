// SPDX-License-Identifier: GPL-3.0-only
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const VPK_SIGNATURE_V1: u32 = 0x55AA1234;
pub const VPK_VERSION_V1: u32 = 1;
pub const VPK_ENTRY_TERMINATOR: u16 = 0xFFFF;
pub const VPK_EMBEDDED_ARCHIVE_INDEX: u16 = 0x7FFF;
/// On-disk v1 header is signature(4) + version(4) + tree_size(4).
pub const VPK_V1_HEADER_SIZE: u64 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpkV1Header {
    pub tree_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpkDirectoryEntry {
    pub crc: u32,
    pub preload_length: u16,
    pub archive_index: u16,
    pub entry_offset: u32,
    pub entry_length: u32,
}

pub fn read_header(file: &mut File) -> anyhow::Result<VpkV1Header> {
    let signature = read_u32_le(file)?;
    let version = read_u32_le(file)?;
    let tree_size = read_u32_le(file)?;

    if signature != VPK_SIGNATURE_V1 {
        anyhow::bail!(
            "Invalid VPK signature: expected {VPK_SIGNATURE_V1:#x}, got {signature:#x}"
        );
    }
    if version != VPK_VERSION_V1 {
        anyhow::bail!("Unsupported VPK version: expected {VPK_VERSION_V1}, got {version}");
    }

    Ok(VpkV1Header { tree_size })
}

/// Walk the VPK v1 directory tree without requiring UTF-8 path strings.
pub fn find_addoninfo_entry(
    file: &mut File,
    header: &VpkV1Header,
) -> anyhow::Result<Option<VpkDirectoryEntry>> {
    let tree_start = file.stream_position()?;
    let tree_end = tree_start + header.tree_size as u64;

    while file.stream_position()? < tree_end {
        let extension = read_null_terminated(file)?;
        if extension.is_empty() {
            break;
        }

        loop {
            let path = read_null_terminated(file)?;
            if path.is_empty() || file.stream_position()? > tree_end {
                break;
            }

            loop {
                let file_name = read_null_terminated(file)?;
                if file_name.is_empty() || file.stream_position()? > tree_end {
                    break;
                }

                let entry = read_directory_entry(file)?;

                if bytes_ieq(&extension, "txt") && bytes_ieq(&file_name, "addoninfo") {
                    return Ok(Some(entry));
                }

                if entry.preload_length > 0 {
                    skip_bytes(file, entry.preload_length as usize)?;
                }
            }
        }
    }

    Ok(None)
}

pub fn read_addoninfo_bytes(
    file: &mut File,
    vpk_path: &Path,
    header: &VpkV1Header,
    entry: &VpkDirectoryEntry,
) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();

    if entry.preload_length > 0 {
        let mut preload = vec![0u8; entry.preload_length as usize];
        file.read_exact(&mut preload)?;
        buf.extend(preload);
    }

    if entry.entry_length > 0 {
        let chunk = if entry.archive_index == VPK_EMBEDDED_ARCHIVE_INDEX {
            let seek_pos = VPK_V1_HEADER_SIZE + header.tree_size as u64 + entry.entry_offset as u64;
            file.seek(SeekFrom::Start(seek_pos))?;
            read_n(file, entry.entry_length as usize)?
        } else {
            read_split_archive_bytes(vpk_path, entry)?
        };
        buf.extend(chunk);
    }

    Ok(buf)
}

fn read_split_archive_bytes(vpk_path: &Path, entry: &VpkDirectoryEntry) -> anyhow::Result<Vec<u8>> {
    let archive_dir = vpk_path.parent().unwrap_or_else(|| Path::new("."));
    let vpk_stem = vpk_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let base_vpk_name = vpk_stem.strip_suffix("_dir").unwrap_or(vpk_stem);

    let chunk_path = if entry.archive_index == 0xFF7F {
        archive_dir.join(format!("{base_vpk_name}_dir.vpk"))
    } else {
        archive_dir.join(format!(
            "{base_vpk_name}_{:03}.vpk",
            entry.archive_index
        ))
    };

    let mut chunk = File::open(&chunk_path)
        .map_err(|e| anyhow::anyhow!("Failed to open VPK chunk {}: {e}", chunk_path.display()))?;

    if entry.archive_index == 0xFF7F {
        let chunk_header = read_header(&mut chunk)?;
        let seek_pos = VPK_V1_HEADER_SIZE
            + chunk_header.tree_size as u64
            + entry.entry_offset as u64;
        chunk.seek(SeekFrom::Start(seek_pos))?;
    } else {
        chunk.seek(SeekFrom::Start(entry.entry_offset as u64))?;
    }

    read_n(&mut chunk, entry.entry_length as usize)
}

fn read_null_terminated(file: &mut File) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte)?;
        if byte[0] == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    Ok(buf)
}

fn read_directory_entry(file: &mut File) -> anyhow::Result<VpkDirectoryEntry> {
    let crc = read_u32_le(file)?;
    let preload_length = read_u16_le(file)?;
    let archive_index = read_u16_le(file)?;
    let entry_offset = read_u32_le(file)?;
    let entry_length = read_u32_le(file)?;
    let terminator = read_u16_le(file)?;

    if terminator != VPK_ENTRY_TERMINATOR {
        anyhow::bail!("Invalid VPK directory entry terminator: {terminator:#x}");
    }

    Ok(VpkDirectoryEntry {
        crc,
        preload_length,
        archive_index,
        entry_offset,
        entry_length,
    })
}

fn read_u16_le(file: &mut File) -> anyhow::Result<u16> {
    let mut buf = [0u8; 2];
    file.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32_le(file: &mut File) -> anyhow::Result<u32> {
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_n(file: &mut File, count: usize) -> anyhow::Result<Vec<u8>> {
    let mut buf = vec![0u8; count];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn skip_bytes(file: &mut File, count: usize) -> anyhow::Result<()> {
    file.seek(SeekFrom::Current(count as i64))?;
    Ok(())
}

fn bytes_ieq(bytes: &[u8], ascii: &str) -> bool {
    bytes.eq_ignore_ascii_case(ascii.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crc::{Crc, CRC_32_ISO_HDLC};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_entry(preload: &[u8]) -> VpkDirectoryEntry {
        let crc_val = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(preload);
        VpkDirectoryEntry {
            crc: crc_val,
            preload_length: preload.len() as u16,
            archive_index: 0,
            entry_offset: 0,
            entry_length: 0,
        }
    }

    fn write_test_vpk_with_bad_path(tree: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&VPK_SIGNATURE_V1.to_le_bytes()).unwrap();
        file.write_all(&VPK_VERSION_V1.to_le_bytes()).unwrap();
        file.write_all(&(tree.len() as u32).to_le_bytes()).unwrap();
        file.write_all(tree).unwrap();
        file
    }

    #[test]
    fn find_addoninfo_skips_non_utf8_paths() {
        let addoninfo_content = b"\"addonTitle\" \"Back To School\"\n\"addonVersion\" \"11\"\n";
        let entry = make_entry(addoninfo_content);

        let mut tree = Vec::new();
        tree.extend_from_slice(b"txt\0");
        tree.extend_from_slice(&[209, 238, 204, 229, 240, 230, 224, 237, 232, 229, 0]);
        tree.extend_from_slice(b"changelog_rus\0");
        tree.extend_from_slice(&0u32.to_le_bytes());
        tree.extend_from_slice(&0u16.to_le_bytes());
        tree.extend_from_slice(&0u16.to_le_bytes());
        tree.extend_from_slice(&0u32.to_le_bytes());
        tree.extend_from_slice(&0u32.to_le_bytes());
        tree.extend_from_slice(&VPK_ENTRY_TERMINATOR.to_le_bytes());
        tree.push(0);
        tree.push(0);

        tree.extend_from_slice(b"txt\0");
        tree.extend_from_slice(b" \0");
        tree.extend_from_slice(b"addoninfo\0");
        tree.extend_from_slice(&entry.crc.to_le_bytes());
        tree.extend_from_slice(&entry.preload_length.to_le_bytes());
        tree.extend_from_slice(&entry.archive_index.to_le_bytes());
        tree.extend_from_slice(&entry.entry_offset.to_le_bytes());
        tree.extend_from_slice(&entry.entry_length.to_le_bytes());
        tree.extend_from_slice(&VPK_ENTRY_TERMINATOR.to_le_bytes());
        tree.extend_from_slice(addoninfo_content);
        tree.push(0);
        tree.push(0);
        tree.push(0);

        let temp = write_test_vpk_with_bad_path(&tree);
        let mut file = File::open(temp.path()).unwrap();
        let header = read_header(&mut file).unwrap();
        let found = find_addoninfo_entry(&mut file, &header)
            .unwrap()
            .expect("addoninfo entry");
        let content = read_addoninfo_bytes(&mut file, temp.path(), &header, &found).unwrap();
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("Back To School"));
    }
}
