// SPDX-License-Identifier: GPL-3.0-only
use url::Url;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use anyhow::{Result, Context};

/// Maximum allowed URL length
const MAX_URL_LENGTH: usize = 2048;

/// Validate a URL to prevent SSRF (Server-Side Request Forgery) attacks
/// 
/// Checks:
/// - Only allows http/https schemes
/// - Rejects private/internal IP addresses
/// - Rejects localhost
/// - Validates URL length
pub fn validate_url(url_str: &str) -> Result<()> {
    // Check URL length
    if url_str.len() > MAX_URL_LENGTH {
        return Err(anyhow::anyhow!("URL exceeds maximum length of {} characters", MAX_URL_LENGTH));
    }
    
    // Parse URL
    let url = Url::parse(url_str)
        .context("Invalid URL format")?;
    
    // Check scheme - only allow http and https
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(anyhow::anyhow!(
                "Invalid URL scheme: {} (only http and https are allowed)",
                scheme
            ));
        }
    }
    
    // Check host
    if let Some(host) = url.host_str() {
        // Check for localhost variants
        if is_localhost(host) {
            return Err(anyhow::anyhow!(
                "URL host is localhost (not allowed for security reasons)"
            ));
        }
        
        // Try to parse as IP address
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(&ip) {
                return Err(anyhow::anyhow!(
                    "URL contains private/internal IP address (not allowed for security reasons)"
                ));
            }
        }
    } else {
        return Err(anyhow::anyhow!("URL must have a host"));
    }
    
    Ok(())
}

/// Check if a hostname is a localhost variant
fn is_localhost(host: &str) -> bool {
    let host_lower = host.to_lowercase();
    matches!(
        host_lower.as_str(),
        "localhost"
        | "127.0.0.1"
        | "::1"
        | "0.0.0.0"
        | "[::1]"
        | "[::]"
    ) || host_lower.starts_with("127.")
}

/// Check if an IP address is private/internal
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => is_private_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_private_ipv6(ipv6),
    }
}

/// Check if an IPv4 address is private/internal
fn is_private_ipv4(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    
    // Private ranges:
    // 10.0.0.0/8
    // 172.16.0.0/12
    // 192.168.0.0/16
    // 169.254.0.0/16 (link-local)
    // 127.0.0.0/8 (loopback)
    
    octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
        || (octets[0] == 192 && octets[1] == 168)
        || (octets[0] == 169 && octets[1] == 254)
        || octets[0] == 127
}

/// Check if an IPv6 address is private/internal
fn is_private_ipv6(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    
    // Private ranges:
    // ::1 (loopback)
    // fc00::/7 (unique local)
    // fe80::/10 (link-local)
    // ::ffff:0:0/96 (IPv4-mapped)
    
    if segments == [0, 0, 0, 0, 0, 0, 0, 1] {
        return true; // ::1
    }
    
    // Check for unique local (fc00::/7)
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    
    // Check for link-local (fe80::/10)
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    
    // Check for IPv4-mapped (::ffff:0:0/96)
    if segments[0] == 0 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0
        && segments[4] == 0 && segments[5] == 0xffff {
        // Extract IPv4 and check if it's private
        let ipv4 = Ipv4Addr::new(
            ((segments[6] >> 8) & 0xff) as u8,
            (segments[6] & 0xff) as u8,
            ((segments[7] >> 8) & 0xff) as u8,
            (segments[7] & 0xff) as u8,
        );
        return is_private_ipv4(&ipv4);
    }
    
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_valid_https() {
        assert!(validate_url("https://example.com/file.zip").is_ok());
    }

    #[test]
    fn test_validate_url_valid_http() {
        assert!(validate_url("http://example.com/file.zip").is_ok());
    }

    #[test]
    fn test_validate_url_invalid_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ftp://example.com/file.zip").is_err());
    }

    #[test]
    fn test_validate_url_localhost() {
        assert!(validate_url("http://localhost/file.zip").is_err());
        assert!(validate_url("http://127.0.0.1/file.zip").is_err());
        assert!(validate_url("http://[::1]/file.zip").is_err());
    }

    #[test]
    fn test_validate_url_private_ip() {
        assert!(validate_url("http://192.168.1.1/file.zip").is_err());
        assert!(validate_url("http://10.0.0.1/file.zip").is_err());
        assert!(validate_url("http://172.16.0.1/file.zip").is_err());
        assert!(validate_url("http://169.254.0.1/file.zip").is_err());
    }

    #[test]
    fn test_validate_url_public_ip() {
        assert!(validate_url("http://8.8.8.8/file.zip").is_ok());
        assert!(validate_url("http://1.1.1.1/file.zip").is_ok());
    }

    #[test]
    fn test_validate_url_too_long() {
        let long_url = format!("https://example.com/{}", "a".repeat(MAX_URL_LENGTH));
        assert!(validate_url(&long_url).is_err());
    }

    #[test]
    fn test_validate_url_invalid_format() {
        assert!(validate_url("not-a-url").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn test_is_localhost() {
        assert!(is_localhost("localhost"));
        assert!(is_localhost("LOCALHOST"));
        assert!(is_localhost("127.0.0.1"));
        assert!(is_localhost("::1"));
        assert!(is_localhost("[::1]"));
        assert!(is_localhost("127.0.0.2"));
        assert!(!is_localhost("example.com"));
    }

    #[test]
    fn test_is_private_ipv4() {
        assert!(is_private_ipv4(&Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(169, 254, 0, 1)));
        assert!(is_private_ipv4(&Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_private_ipv4(&Ipv4Addr::new(8, 8, 8, 8)));
    }
}

