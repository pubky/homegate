use axum::http::HeaderMap;
use std::net::SocketAddr;

/// Extract client IP address from request, checking proxy headers first
pub fn extract_client_ip(addr: SocketAddr, headers: &HeaderMap) -> String {
    // Check X-Forwarded-For header first (standard proxy header)
    if let Some(forwarded) = headers.get("x-forwarded-for")
        && let Ok(value) = forwarded.to_str()
    {
        // Take first IP in comma-separated list (original client)
        if let Some(ip) = value.split(',').next() {
            let trimmed = ip.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    // Check X-Real-IP header (alternative proxy header)
    if let Some(real_ip) = headers.get("x-real-ip")
        && let Ok(value) = real_ip.to_str()
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // Fallback to direct TCP connection IP
    addr.ip().to_string()
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

        let result = extract_client_ip(addr, &headers);
        assert_eq!(result, "203.0.113.1");
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

        let result = extract_client_ip(addr, &headers);
        // Should extract the first IP (original client)
        assert_eq!(result, "203.0.113.1");
    }

    #[test]
    fn test_ip_from_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_client_ip(addr, &headers);
        assert_eq!(result, "203.0.113.2");
    }

    #[test]
    fn test_x_forwarded_for_precedence_over_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let result = extract_client_ip(addr, &headers);
        // X-Forwarded-For should take precedence
        assert_eq!(result, "203.0.113.1");
    }

    #[test]
    fn test_ip_fallback_to_socket_address() {
        let headers = HeaderMap::new();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let result = extract_client_ip(addr, &headers);
        // Should fall back to socket address
        assert_eq!(result, "192.168.1.100");
    }

    #[test]
    fn test_malformed_forwarded_for_header() {
        let mut headers = HeaderMap::new();
        // Empty value
        headers.insert("x-forwarded-for", "".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let result = extract_client_ip(addr, &headers);
        // Should fall back to socket address when header is empty
        assert_eq!(result, "192.168.1.100");
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

        let result = extract_client_ip(addr, &headers);
        // Should trim whitespace
        assert_eq!(result, "203.0.113.1");
    }
}
