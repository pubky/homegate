use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};

use crate::infrastructure::{
    config::IpVerificationConfig,
    http::{HttpServerError, RequestOrigin},
};
use crate::shared::HomeserverAdminAPI;

use super::app_state::AppState;
use super::error::IpVerificationError;
use super::types::IpVerificationResponse;

pub async fn router(
    homeserver_api: &HomeserverAdminAPI,
    ip: &IpVerificationConfig,
    db: crate::infrastructure::sql::SqlDb,
    hasher: crate::shared::HasherArgon2id,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(homeserver_api, ip, db, hasher);
    Ok(Router::new()
        .route("/", post(root_handler))
        .with_state(state))
}

async fn root_handler(
    State(state): State<AppState>,
    RequestOrigin(maybe_ip): RequestOrigin,
) -> Result<Json<IpVerificationResponse>, IpVerificationError> {
    let ip_address = maybe_ip.ok_or(IpVerificationError::IpAddressRequired)?;
    let response = state.ip_verification.verify(ip_address).await?;
    Ok(Json(response))
}

impl IntoResponse for IpVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            IpVerificationError::WeeklyLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            IpVerificationError::AnnualLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            IpVerificationError::IpAddressRequired => StatusCode::BAD_REQUEST,
            IpVerificationError::HomeserverUnavailable => {
                tracing::error!("Homeserver unavailable during IP verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            IpVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::{WiremockServers, setup_homeserver_signup_token};
    use crate::infrastructure::config::IpVerificationConfig;
    use crate::shared::HomeserverAdminAPI;
    use axum_test::TestServer;
    use sqlx::PgPool;
    use std::net::SocketAddr;

    async fn create_http_test_server(pool: PgPool, servers: &WiremockServers) -> TestServer {
        create_http_test_server_with_limits(pool, servers, 2, 4).await
    }

    async fn create_http_test_server_with_limits(
        pool: PgPool,
        servers: &WiremockServers,
        max_per_week: u32,
        max_per_year: u32,
    ) -> TestServer {
        create_test_server_inner(pool, servers, max_per_week, max_per_year, true).await
    }

    async fn create_test_server_inner(
        pool: PgPool,
        servers: &WiremockServers,
        max_per_week: u32,
        max_per_year: u32,
        with_connect_info: bool,
    ) -> TestServer {
        use crate::infrastructure::sql::SqlDb;

        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
        );
        let ip_config = IpVerificationConfig {
            max_verifications_per_week: max_per_week,
            max_verifications_per_year: max_per_year,
            signup_quota: None,
            limit_whitelist: vec![],
        };

        let db = SqlDb::test(pool).await;

        let hasher = crate::shared::HasherArgon2id::new(
            tempfile::tempdir().unwrap().keep().join("pepper.txt"),
        );
        let ip_verification_router = router(&homeserver_api, &ip_config, db, hasher)
            .await
            .expect("Failed to create router");

        let router = Router::new().nest("/ip_verification", ip_verification_router);
        if with_connect_info {
            let app = router.into_make_service_with_connect_info::<SocketAddr>();
            TestServer::new(app).expect("Failed to create test server")
        } else {
            let app = router.into_make_service();
            TestServer::new(app).expect("Failed to create test server")
        }
    }

    #[sqlx::test]
    async fn test_successful_verification(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers).await;

        setup_homeserver_signup_token("token-123")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server.post("/ip_verification").await;
        response.assert_status_ok();

        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "token-123");
        assert_eq!(body["homeserverPubky"], "test-homeserver-pubky");
    }

    #[sqlx::test]
    async fn test_weekly_limit_exceeded(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers).await;

        // max_ip_verifications_per_week is 2, so third request should fail.
        // The homeserver is only called for requests that pass rate limits.
        setup_homeserver_signup_token("token")
            .expect(2)
            .mount(&servers.homeserver_server)
            .await;

        server.post("/ip_verification").await.assert_status_ok();
        server.post("/ip_verification").await.assert_status_ok();

        let response = server.post("/ip_verification").await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);
        let body = response.text();
        assert!(
            body.contains("weekly"),
            "Should mention weekly limit: {body}"
        );
    }

    #[sqlx::test]
    async fn test_annual_limit_exceeded(pool: PgPool) {
        let servers = WiremockServers::start().await;
        // Set weekly limit high (10) so only the annual limit (2) triggers.
        // The homeserver is only called for requests that pass rate limits.
        let server = create_http_test_server_with_limits(pool, &servers, 10, 2).await;

        setup_homeserver_signup_token("token")
            .expect(2)
            .mount(&servers.homeserver_server)
            .await;

        server.post("/ip_verification").await.assert_status_ok();
        server.post("/ip_verification").await.assert_status_ok();

        let response = server.post("/ip_verification").await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);
        let body = response.text();
        assert!(
            body.contains("annual"),
            "Should mention annual limit: {body}"
        );
    }

    #[sqlx::test]
    async fn test_homeserver_unavailable_returns_500(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers).await;

        // Don't mount any homeserver mock — requests to generate_signup_token will get 404
        let response = server.post("/ip_verification").await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.text();
        assert!(
            body.contains("Homeserver temporarily unavailable"),
            "Should mention homeserver unavailable: {body}"
        );
    }

    #[sqlx::test]
    async fn test_missing_ip_returns_400(pool: PgPool) {
        let servers = WiremockServers::start().await;
        // Build without ConnectInfo so IP will be None
        let server = create_test_server_inner(pool, &servers, 2, 4, false).await;

        let response = server.post("/ip_verification").await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body = response.text();
        assert!(
            body.contains("IP address"),
            "Should mention IP address: {body}"
        );
    }
}
