use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::shared::HomeserverAdminAPI;

/// Middleware that checks homeserver health before processing the request.
/// Returns 503 Service Unavailable if the homeserver is down.
pub async fn check_homeserver_health(
    homeserver_api: HomeserverAdminAPI,
    request: Request,
    next: Next,
) -> Response {
    if let Err(e) = homeserver_api.health_check().await {
        tracing::warn!("Homeserver unavailable: {:?}", e);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Homeserver temporarily unavailable, please retry",
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, middleware, response::IntoResponse, routing::get};
    use axum_test::TestServer;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    // Simple handler that returns 200 OK for testing
    async fn test_handler() -> impl IntoResponse {
        (StatusCode::OK, "Handler reached")
    }

    /// Helper to create a router with the homeserver health middleware applied
    fn create_test_router(homeserver_api: HomeserverAdminAPI) -> Router {
        Router::new()
            .route("/test", get(test_handler))
            .route_layer(middleware::from_fn(move |req, next| {
                let api = homeserver_api.clone();
                check_homeserver_health(api, req, next)
            }))
    }

    #[tokio::test]
    async fn test_middleware_passes_through_when_homeserver_healthy() {
        // Setup wiremock homeserver that returns 200 OK
        let homeserver_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
            .expect(1)
            .mount(&homeserver_server)
            .await;

        let homeserver_api = HomeserverAdminAPI::new(
            &homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-pubky",
        );

        let app = create_test_router(homeserver_api);
        let server = TestServer::new(app).expect("Failed to create test server");

        // Make request through middleware
        let response = server.get("/test").await;

        // Verify request passed through to handler
        response.assert_status_ok();
        assert_eq!(response.text(), "Handler reached");
    }

    #[tokio::test]
    async fn test_middleware_returns_503_when_homeserver_returns_500() {
        // Setup wiremock homeserver that returns 500 Internal Server Error
        let homeserver_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&homeserver_server)
            .await;

        let homeserver_api = HomeserverAdminAPI::new(
            &homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-pubky",
        );

        let app = create_test_router(homeserver_api);
        let server = TestServer::new(app).expect("Failed to create test server");

        // Make request through middleware
        let response = server.get("/test").await;

        // Verify middleware returned 503
        response.assert_status(StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.text(),
            "Homeserver temporarily unavailable, please retry"
        );
    }

    #[tokio::test]
    async fn test_middleware_returns_503_when_homeserver_unreachable() {
        // Create homeserver API pointing to a non-existent server
        // Using an invalid URL that will fail to connect
        let homeserver_api = HomeserverAdminAPI::new(
            &"http://localhost:1".parse().unwrap(), // Port 1 is typically unreachable
            "test-pass",
            "test-pubky",
        );

        let app = create_test_router(homeserver_api);
        let server = TestServer::new(app).expect("Failed to create test server");

        // Make request through middleware
        let response = server.get("/test").await;

        // Verify middleware returned 503 when connection fails
        response.assert_status(StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.text(),
            "Homeserver temporarily unavailable, please retry"
        );
    }
}
