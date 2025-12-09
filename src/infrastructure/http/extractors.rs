use axum::{
    extract::{ConnectInfo, FromRequest, FromRequestParts, Request},
    http::{HeaderMap, request::Parts},
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;
use std::marker::PhantomData;
use std::net::{IpAddr, SocketAddr};

// Inspired by https://github.com/benwis/tower-governor/blob/main/src/key_extractor.rs#L121
const X_REAL_IP: &str = "x-real-ip";
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Axum extractor for RequestOrigin (client IP address) with proxy header support
///
/// Extracts the client's IP address by checking proxy headers (X-Forwarded-For, X-Real-IP)
/// and falling back to the direct socket address if headers are not present.
pub struct RequestOrigin(pub IpAddr);

fn maybe_x_forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_FORWARDED_FOR)
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.split(',').find_map(|s| s.trim().parse::<IpAddr>().ok()))
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
        let headers = HeaderMap::from_request_parts(parts, state)
            .await
            .expect("HeaderMap extractor should never fail");
        let ConnectInfo(addr) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .expect("ConnectInfo extractor should never fail");
        let ip = maybe_x_forwarded_for(&headers)
            .or_else(|| maybe_x_real_ip(&headers))
            .unwrap_or_else(|| addr.ip());
        Ok(RequestOrigin(ip))
    }
}

/// Generic extractor that deserializes JSON and validates it using TryFrom
///
/// The reason for this is so that our Http Request parameters can be validated as part of deserialisation.
/// Without this, a failed validation returns an error string, instead of the desired Http Response JSON error.
///
/// The type `T` must implement `TryFrom<Raw>` where `Raw` is deserializable from JSON.
pub struct ValidatedJson<T, Raw, E>(T, PhantomData<(Raw, E)>)
where
    Raw: DeserializeOwned,
    T: TryFrom<Raw, Error = E>,
    E: IntoResponse;

impl<T, Raw, E> ValidatedJson<T, Raw, E>
where
    Raw: DeserializeOwned,
    T: TryFrom<Raw, Error = E>,
    E: IntoResponse,
{
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T, Raw, E, S> FromRequest<S> for ValidatedJson<T, Raw, E>
where
    Raw: DeserializeOwned,
    T: TryFrom<Raw, Error = E>,
    E: IntoResponse,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        // First, deserialize the JSON into the raw type
        // Preserve Axum's default rejection handling (eg 415 for malformed JSON or 422 for validation errors)
        let axum::Json(raw) = axum::Json::<Raw>::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;
        // Then, validate and convert to the final type
        let validated = T::try_from(raw).map_err(|e| e.into_response())?;
        Ok(ValidatedJson(validated, PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, Request};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    // Helper function to create a mock request with headers and ConnectInfo
    fn make_request(headers: HeaderMap, addr: SocketAddr) -> Request<()> {
        let mut request = Request::builder().body(()).unwrap();
        *request.headers_mut() = headers;
        request.extensions_mut().insert(ConnectInfo(addr));
        request
    }

    #[tokio::test]
    async fn test_ip_from_x_forwarded_for_single() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn test_ip_from_x_forwarded_for_multiple() {
        let mut headers = HeaderMap::new();
        // Multiple IPs: client, proxy1, proxy2
        headers.insert(
            "x-forwarded-for",
            "203.0.113.1, 198.51.100.1, 192.0.2.1".parse().unwrap(),
        );
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should extract the first IP (original client)
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn test_ip_from_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        assert_eq!(result, "203.0.113.2".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn test_x_forwarded_for_precedence_over_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // X-Forwarded-For should take precedence
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn test_ip_fallback_to_socket_address() {
        let headers = HeaderMap::new();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should fall back to socket address
        assert_eq!(result, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)));
    }

    #[tokio::test]
    async fn test_malformed_forwarded_for_header() {
        let mut headers = HeaderMap::new();
        // Empty value
        headers.insert("x-forwarded-for", "".parse().unwrap());
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should fall back to socket address when header is empty
        assert_eq!(result, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)));
    }

    #[tokio::test]
    async fn test_x_forwarded_for_with_whitespace() {
        let mut headers = HeaderMap::new();
        // IPs with extra whitespace
        headers.insert(
            "x-forwarded-for",
            "  203.0.113.1  ,  198.51.100.1  ".parse().unwrap(),
        );
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);

        let request = make_request(headers, addr);
        let (mut parts, _) = request.into_parts();
        let RequestOrigin(result) = RequestOrigin::from_request_parts(&mut parts, &())
            .await
            .unwrap();
        // Should trim whitespace
        assert_eq!(result, "203.0.113.1".parse::<IpAddr>().unwrap());
    }
}
