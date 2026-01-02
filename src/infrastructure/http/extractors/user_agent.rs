use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};
/// Axum extractor for User-Agent header
///
/// Extracts the User-Agent header from HTTP requests.
/// Returns None if the header is missing or cannot be parsed as a valid string.
pub struct UserAgent(pub Option<String>);

impl<S> FromRequestParts<S> for UserAgent
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let headers = HeaderMap::from_request_parts(parts, state).await?;

        let user_agent = headers
            .get("user-agent")
            .and_then(|hv| hv.to_str().ok())
            .map(|s| s.to_string());

        Ok(UserAgent(user_agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Request, http::HeaderMap};

    // Helper function to create a mock request with headers
    fn make_request(headers: HeaderMap) -> Request<()> {
        let mut request = Request::builder().body(()).unwrap();
        *request.headers_mut() = headers;
        request
    }

    #[tokio::test]
    async fn test_user_agent_present() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "user-agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36"
                .parse()
                .unwrap(),
        );

        let request = make_request(headers);
        let (mut parts, _) = request.into_parts();
        let UserAgent(result) = UserAgent::from_request_parts(&mut parts, &())
            .await
            .unwrap();

        assert_eq!(
            result,
            Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36".to_string())
        );
    }
}
