// SPDX-License-Identifier: GPL-3.0-only
use regex::Regex;
use url::Url;

#[derive(Debug, Clone)]
pub enum UrlType {
    WorkshopId(u64),
    ZipUrl(String),
}

pub fn parse_url(url_or_id: &str) -> anyhow::Result<UrlType> {
    // Try to parse as numeric workshop ID first
    if let Ok(workshop_id) = url_or_id.parse::<u64>() {
        // Check if it looks like a workshop ID (reasonable range)
        if workshop_id > 1000 && workshop_id < u64::MAX {
            return Ok(UrlType::WorkshopId(workshop_id));
        }
    }
    
    // Try to parse as URL
    if let Ok(parsed_url) = Url::parse(url_or_id) {
        // Check if it's a Steam Workshop URL
        if parsed_url.host_str().map(|h| h.contains("steamcommunity.com")).unwrap_or(false) {
            if parsed_url.path().contains("/sharedfiles/filedetails/") {
                // Extract workshop ID from query parameters
                if let Some(id_param) = parsed_url.query_pairs().find(|(key, _)| key == "id") {
                    if let Ok(workshop_id) = id_param.1.parse::<u64>() {
                        return Ok(UrlType::WorkshopId(workshop_id));
                    }
                }
            }
        }
        
        // Check if URL ends with .zip or .vpk
        let path = parsed_url.path();
        if path.ends_with(".zip") || path.ends_with(".ZIP") {
            return Ok(UrlType::ZipUrl(url_or_id.to_string()));
        }
        
        // Default to ZIP URL if it's a valid HTTP/HTTPS URL
        if parsed_url.scheme() == "http" || parsed_url.scheme() == "https" {
            return Ok(UrlType::ZipUrl(url_or_id.to_string()));
        }
    }
    
    // Try regex pattern for workshop URLs
    let workshop_re = Regex::new(r"(?:steamcommunity\.com/sharedfiles/filedetails/.*[?&]id=(\d+)|workshop.*[?&]id=(\d+))")?;
    if let Some(caps) = workshop_re.captures(url_or_id) {
        if let Some(id_str) = caps.get(1).or_else(|| caps.get(2)) {
            if let Ok(workshop_id) = id_str.as_str().parse::<u64>() {
                return Ok(UrlType::WorkshopId(workshop_id));
            }
        }
    }
    
    // Default: assume it's a ZIP URL
    Ok(UrlType::ZipUrl(url_or_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_numeric_workshop_id() {
        assert!(matches!(parse_url("123456789").unwrap(), UrlType::WorkshopId(123456789)));
    }
    
    #[test]
    fn test_parse_workshop_url() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=123456789";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(123456789)));
    }
    
    #[test]
    fn test_parse_zip_url() {
        let url = "https://example.com/map.zip";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
}

