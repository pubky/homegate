use axum::http::HeaderMap;
use std::net::{IpAddr, SocketAddr};

// From https://github.com/benwis/tower-governor/blob/main/src/key_extractor.rs#L121
const X_REAL_IP: &str = "x-real-ip";
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Tries to parse the `x-forwarded-for` header
fn maybe_x_forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_FORWARDED_FOR)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.split(',').find_map(|s| s.trim().parse::<IpAddr>().ok()))
}

/// Tries to parse the `x-real-ip` header
fn maybe_x_real_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_REAL_IP)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.parse::<IpAddr>().ok())
}

/// Extract the client IP address from headers and socket address
///
/// This is a convenience function for use in Axum handlers where you have
/// separate extractors for headers and connection info.
///
/// # Arguments
/// * `headers` - The HTTP request headers
/// * `addr` - The socket address from ConnectInfo
/// * `behind_proxy` - Whether to trust proxy headers (X-Forwarded-For, X-Real-IP)
///
/// # Security
/// - When `behind_proxy` is `false`: Only uses the direct socket IP
/// - When `behind_proxy` is `true`: Trusts proxy headers, which can be spoofed by attackers if misconfigured
///
/// # Returns
/// The client's IP address for rate limiting
pub fn extract_ip(
    headers: &HeaderMap,
    addr: SocketAddr,
    behind_proxy: bool,
) -> IpAddr {
    if behind_proxy {
        // Trust proxy headers and direct socket IP
        maybe_x_forwarded_for(headers)
            .or_else(|| maybe_x_real_ip(headers))
            .unwrap_or_else(|| addr.ip())
    } else {
        // Only trust the direct socket IP
        addr.ip()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn test_ip_from_x_forwarded_for_single() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_ip(&headers, addr, true);
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_ip_from_x_forwarded_for_multiple() {
        let mut headers = HeaderMap::new();
        // Multiple IPs: client, proxy1, proxy2
        headers.insert(
            "x-forwarded-for",
            "203.0.113.1, 198.51.100.1, 192.0.2.1".parse().unwrap(),
        );
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_ip(&headers, addr, true);
        // Should extract the first IP (original client)
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_ip_from_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_ip(&headers, addr, true);
        assert_eq!(result, "203.0.113.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_x_forwarded_for_precedence_over_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_ip(&headers, addr, true);
        // X-Forwarded-For should take precedence
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_ip_fallback_to_socket_address() {
        let headers = HeaderMap::new();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let result = extract_ip(&headers, addr, true);
        // Should fall back to socket address
        assert_eq!(result, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)));
    }

    #[test]
    fn test_malformed_forwarded_for_header() {
        let mut headers = HeaderMap::new();
        // Empty value
        headers.insert("x-forwarded-for", "".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let result = extract_ip(&headers, addr, true);
        // Should fall back to socket address when header is empty
        assert_eq!(result, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)));
    }

    #[test]
    fn test_x_forwarded_for_with_whitespace() {
        let mut headers = HeaderMap::new();
        // IPs with extra whitespace
        headers.insert(
            "x-forwarded-for",
            "  203.0.113.1  ,  198.51.100.1  ".parse().unwrap(),
        );
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_ip(&headers, addr, true);
        // Should trim whitespace
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_behind_proxy_false_ignores_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let result = extract_ip(&headers, addr, false);
        // Should only use socket address when behind_proxy is false
        assert_eq!(result, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)));
    }
}
