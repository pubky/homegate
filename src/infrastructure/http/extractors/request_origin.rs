use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderMap, request::Parts},
};
use std::net::{IpAddr, SocketAddr};

// Inspired by https://github.com/benwis/tower-governor/blob/main/src/key_extractor.rs#L121
const X_REAL_IP: &str = "x-real-ip";
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Extension marker that enables trusting X-Forwarded-For / X-Real-IP headers.
///
/// When this is present in the request extensions (added via router layer),
/// the extractor will check proxy headers before falling back to the socket
/// address. Without it, only the socket address is used.
#[derive(Clone, Copy, Debug)]
pub struct AcceptProxyIpHeaders;

/// Axum extractor for RequestOrigin (client IP address).
///
/// By default, only uses the socket address. If [`AcceptProxyIpHeaders`] is
/// present as a request extension, also checks X-Forwarded-For and X-Real-IP.
pub struct RequestOrigin(pub Option<IpAddr>);

/// Extract the client IP from X-Forwarded-For using the **rightmost** valid IP.
///
/// The rightmost entry is the one added by the closest trusted proxy and is
/// the hardest for a client to spoof. If the proxy appends `$remote_addr`,
/// the rightmost IP is the true client address regardless of what the
/// client put in the header.
fn maybe_x_forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_FORWARDED_FOR)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.rsplit(',').find_map(|s| s.trim().parse::<IpAddr>().ok()))
}

fn maybe_x_real_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_REAL_IP)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.parse::<IpAddr>().ok())
}

impl<S> FromRequestParts<S> for RequestOrigin
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let socket_ip = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|ConnectInfo(addr)| addr.ip());

        let ip = if parts.extensions.get::<AcceptProxyIpHeaders>().is_some() {
            let headers = HeaderMap::from_request_parts(parts, state).await?;
            maybe_x_forwarded_for(&headers)
                .or_else(|| maybe_x_real_ip(&headers))
                .or(socket_ip)
        } else {
            socket_ip
        };

        Ok(RequestOrigin(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, Request};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn make_request(headers: HeaderMap, addr: SocketAddr, accept_proxy: bool) -> Request<()> {
        let mut request = Request::builder().body(()).unwrap();
        *request.headers_mut() = headers;
        request.extensions_mut().insert(ConnectInfo(addr));
        if accept_proxy {
            request.extensions_mut().insert(AcceptProxyIpHeaders);
        }
        request
    }

    #[tokio::test]
    async fn test_proxy_headers_ignored_by_default() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr, false);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should use socket address, ignoring the proxy header
        assert_eq!(result, Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    }

    #[tokio::test]
    async fn test_ip_from_x_forwarded_for_single() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, Some("203.0.113.1".parse::<IpAddr>().unwrap()));
    }

    #[tokio::test]
    async fn test_ip_from_x_forwarded_for_multiple() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.1, 198.51.100.1, 192.0.2.1".parse().unwrap(),
        );
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should extract the rightmost IP (closest trusted proxy entry)
        assert_eq!(result, Some("192.0.2.1".parse::<IpAddr>().unwrap()));
    }

    #[tokio::test]
    async fn test_ip_from_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, Some("203.0.113.2".parse::<IpAddr>().unwrap()));
    }

    #[tokio::test]
    async fn test_x_forwarded_for_precedence_over_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // X-Forwarded-For should take precedence
        assert_eq!(result, Some("203.0.113.1".parse::<IpAddr>().unwrap()));
    }

    #[tokio::test]
    async fn test_socket_fallback_when_proxy_enabled_but_no_headers() {
        let headers = HeaderMap::new();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should fall back to socket address
        assert_eq!(result, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))));
    }

    #[tokio::test]
    async fn test_socket_address_used_without_proxy_flag() {
        let headers = HeaderMap::new();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let request = make_request(headers, addr, false);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))));
    }

    #[tokio::test]
    async fn test_malformed_forwarded_for_falls_back_to_socket() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))));
    }

    #[tokio::test]
    async fn test_x_forwarded_for_with_whitespace() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "  203.0.113.1  ,  198.51.100.1  ".parse().unwrap(),
        );
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr, true);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Rightmost IP after trimming whitespace
        assert_eq!(result, Some("198.51.100.1".parse::<IpAddr>().unwrap()));
    }

    #[tokio::test]
    async fn test_no_ip_available_returns_none() {
        let headers = HeaderMap::new();
        // Build a request without ConnectInfo
        let mut request = Request::builder().body(()).unwrap();
        *request.headers_mut() = headers;

        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, None);
    }
}
