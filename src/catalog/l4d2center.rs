// SPDX-License-Identifier: GPL-3.0-only
use serde::{Deserialize, Serialize};

use crate::registry::traits::Registry;
use crate::utils::{md5_matches, validate_url};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogMapStatus {
    NotInstalled,
    UpToDate,
    Outdated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L4d2CenterIndexEntry {
    pub name: String,
    pub size: u64,
    pub md5: String,
    pub download_link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L4d2CenterCatalogEntry {
    pub name: String,
    pub size: u64,
    pub md5: String,
    pub download_link: String,
    pub installed: bool,
    pub map_id: Option<u64>,
    pub status: CatalogMapStatus,
}

pub async fn fetch_index(url: &str) -> anyhow::Result<Vec<L4d2CenterIndexEntry>> {
    validate_url(url).map_err(|e| anyhow::anyhow!("Invalid L4D2Center index URL: {e}"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to fetch L4D2Center index: HTTP {}",
            response.status()
        );
    }

    let entries: Vec<L4d2CenterIndexEntry> = response.json().await?;
    Ok(entries)
}

pub fn find_index_entry<'a>(
    entries: &'a [L4d2CenterIndexEntry],
    name: &str,
) -> Option<&'a L4d2CenterIndexEntry> {
    entries.iter().find(|entry| entry.name == name)
}

pub async fn enrich_with_registry(
    entries: Vec<L4d2CenterIndexEntry>,
    registry: &dyn Registry,
) -> anyhow::Result<Vec<L4d2CenterCatalogEntry>> {
    let maps = registry.list_maps().await?;

    Ok(entries
        .into_iter()
        .map(|entry| {
            let installed_map = maps
                .iter()
                .find(|map| map.installed_path == entry.name);

            let (installed, map_id, status) = match installed_map {
                None => (false, None, CatalogMapStatus::NotInstalled),
                Some(map) => {
                    let up_to_date = map
                        .checksum
                        .as_deref()
                        .is_some_and(|checksum| md5_matches(checksum, &entry.md5));
                    (
                        true,
                        Some(map.id),
                        if up_to_date {
                            CatalogMapStatus::UpToDate
                        } else {
                            CatalogMapStatus::Outdated
                        },
                    )
                }
            };

            L4d2CenterCatalogEntry {
                name: entry.name,
                size: entry.size,
                md5: entry.md5,
                download_link: entry.download_link,
                installed,
                map_id,
                status,
            }
        })
        .collect())
}

/// Encode spaces in download URLs the same way as the upstream bash sync script.
pub fn encode_download_url(url: &str) -> String {
    url.replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_INDEX: &str = r#"[
        {
            "name": "widebox1.vpk",
            "size": 10166415,
            "md5": "b0bd409a4bbc4a61ae000c73a9aa9934",
            "download_link": "https://l4d2center.com/maps/servers/widebox1.7z"
        },
        {
            "name": "dark carnival remix.vpk",
            "size": 491903871,
            "md5": "5da54e4ecd9c33403847a5d733ab2cf1",
            "download_link": "https://l4d2center.com/maps/servers/dark carnival remix.7z"
        }
    ]"#;

    #[test]
    fn parse_sample_index_json() {
        let entries: Vec<L4d2CenterIndexEntry> = serde_json::from_str(SAMPLE_INDEX).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "widebox1.vpk");
        assert_eq!(entries[1].download_link, "https://l4d2center.com/maps/servers/dark carnival remix.7z");
    }

    #[test]
    fn find_index_entry_by_exact_name() {
        let entries: Vec<L4d2CenterIndexEntry> = serde_json::from_str(SAMPLE_INDEX).unwrap();
        assert!(find_index_entry(&entries, "widebox1.vpk").is_some());
        assert!(find_index_entry(&entries, "missing.vpk").is_none());
    }

    #[test]
    fn encode_download_url_spaces() {
        assert_eq!(
            encode_download_url("https://example.com/dark carnival remix.7z"),
            "https://example.com/dark%20carnival%20remix.7z"
        );
    }
}
