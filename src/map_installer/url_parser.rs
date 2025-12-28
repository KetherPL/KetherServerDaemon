// SPDX-License-Identifier: GPL-3.0-only
use anyhow::Context;
use regex::Regex;
use url::Url;

#[derive(Debug, Clone)]
pub enum UrlType {
    WorkshopId(u64),
    ZipUrl(String),
}

pub fn parse_url(url_or_id: &str) -> anyhow::Result<UrlType> {
    // Validate URL length first
    if url_or_id.len() > 2048 {
        return Err(anyhow::anyhow!("URL exceeds maximum length of 2048 characters"));
    }
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
        // But validate it first to prevent SSRF attacks
        if parsed_url.scheme() == "http" || parsed_url.scheme() == "https" {
            // Validate URL to prevent SSRF
            crate::utils::validate_url(url_or_id)
                .context("URL validation failed (possible SSRF attempt)")?;
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
    fn test_parse_numeric_workshop_id_boundary_valid() {
        // Minimum valid (just above 1000)
        assert!(matches!(parse_url("1001").unwrap(), UrlType::WorkshopId(1001)));
        // Large valid ID
        assert!(matches!(parse_url("999999999999999999").unwrap(), UrlType::WorkshopId(999999999999999999)));
    }
    
    #[test]
    fn test_parse_numeric_workshop_id_too_small() {
        // Too small (< 1000) should default to ZIP URL
        let result = parse_url("999").unwrap();
        match result {
            UrlType::ZipUrl(_) => {} // Expected
            _ => panic!("Expected ZipUrl for ID < 1000"),
        }
    }
    
    #[test]
    fn test_parse_workshop_url_https() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=123456789";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(123456789)));
    }
    
    #[test]
    fn test_parse_workshop_url_http() {
        let url = "http://steamcommunity.com/sharedfiles/filedetails/?id=987654321";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(987654321)));
    }
    
    #[test]
    fn test_parse_workshop_url_with_additional_params() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=555555555&searchtext=test";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(555555555)));
    }
    
    #[test]
    fn test_parse_workshop_url_with_ampersand_first() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?searchtext=test&id=444444444";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(444444444)));
    }
    
    #[test]
    fn test_parse_workshop_url_with_hash() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=333333333#comments";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(333333333)));
    }
    
    #[test]
    fn test_parse_zip_url_lowercase() {
        let url = "https://example.com/map.zip";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_zip_url_uppercase() {
        let url = "https://example.com/map.ZIP";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_zip_url_http() {
        let url = "http://example.com/map.zip";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_zip_url_with_path() {
        let url = "https://example.com/maps/custom/test_map.zip";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_zip_url_with_query_params() {
        let url = "https://example.com/download?file=map.zip";
        let result = parse_url(url).unwrap();
        match result {
            UrlType::ZipUrl(url_str) => assert!(url_str.contains("example.com")),
            _ => panic!("Expected ZipUrl"),
        }
    }
    
    #[test]
    fn test_parse_generic_https_url_as_zip() {
        let url = "https://example.com/download";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_invalid_url_string_as_zip() {
        // Invalid URLs should default to ZipUrl
        let url = "not-a-valid-url-at-all";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_empty_string_as_zip() {
        // Empty string should default to ZipUrl
        let url = "";
        assert!(matches!(parse_url(url).unwrap(), UrlType::ZipUrl(_)));
    }
    
    #[test]
    fn test_parse_workshop_url_malformed_id() {
        // URL with non-numeric ID should fall through to ZipUrl
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=abc123";
        let result = parse_url(url).unwrap();
        match result {
            UrlType::ZipUrl(_) => {} // Expected - malformed ID
            _ => panic!("Expected ZipUrl for malformed workshop ID"),
        }
    }
    
    #[test]
    fn test_parse_workshop_url_no_id_param() {
        // Workshop URL without ID parameter should be treated as ZIP
        let url = "https://steamcommunity.com/sharedfiles/filedetails/";
        let result = parse_url(url).unwrap();
        match result {
            UrlType::ZipUrl(_) => {} // Expected
            _ => panic!("Expected ZipUrl for workshop URL without ID"),
        }
    }
    
    #[test]
    fn test_parse_workshop_url_regex_fallback() {
        // Test regex fallback for unusual workshop URL formats
        let url = "steamcommunity.com/sharedfiles/filedetails/?id=222222222";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(222222222)));
    }
    
    #[test]
    fn test_parse_workshop_id_extraction_from_regex() {
        // Test regex pattern matching
        let url = "workshop?id=111111111";
        assert!(matches!(parse_url(url).unwrap(), UrlType::WorkshopId(111111111)));
    }
}

