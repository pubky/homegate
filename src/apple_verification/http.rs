use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};

use crate::infrastructure::{config::AppleVerificationConfig, http::HttpServerError, sql::SqlDb};
use crate::shared::{HasherArgon2id, HomeserverAdminAPI};

use super::app_state::AppState;
use super::error::AppleVerificationError;
use super::types::{AppleVerificationRequest, AppleVerificationResponse};

pub async fn router(
    homeserver_api: &HomeserverAdminAPI,
    apple: &AppleVerificationConfig,
    db: SqlDb,
    hasher: HasherArgon2id,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(homeserver_api, apple, db, hasher);
    Ok(Router::new()
        .route("/", post(root_handler))
        .with_state(state))
}

async fn root_handler(
    State(state): State<AppState>,
    Json(request): Json<AppleVerificationRequest>,
) -> Result<Json<AppleVerificationResponse>, AppleVerificationError> {
    let response = state
        .apple_verification
        .verify(&request.apple_id_token)
        .await?;
    Ok(Json(response))
}

impl IntoResponse for AppleVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            AppleVerificationError::InvalidAppleIdToken => StatusCode::UNAUTHORIZED,
            AppleVerificationError::WeeklyLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            AppleVerificationError::AnnualLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            AppleVerificationError::HomeserverUnavailable => {
                tracing::error!("Homeserver unavailable during Apple verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppleVerificationError::AppleVerifierUnavailable => {
                tracing::error!("Apple verifier unavailable during Apple verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppleVerificationError::Database(ref error) => {
                tracing::error!(error = %error, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::routing::post;
    use axum_test::TestServer;
    use sqlx::PgPool;
    use wiremock::MockServer;

    use super::*;
    use crate::apple_verification::apple_id_token_verifier::test_support::{
        TEST_APPLE_AUDIENCE, jwks_server, jwks_url, valid_token, wrong_audience_token,
    };
    use crate::apple_verification::service::AppleVerificationService;
    use crate::e2e::{WiremockServers, setup_homeserver_signup_token};
    use crate::infrastructure::config::{AppConfig, AppleVerificationConfig};
    use crate::infrastructure::http::HttpServer;
    use crate::infrastructure::sql::SqlDb;
    use crate::shared::HomeserverAdminAPI;

    async fn create_http_test_server(
        pool: PgPool,
        servers: &WiremockServers,
        apple_server: &MockServer,
        max_per_week: u32,
    ) -> TestServer {
        create_http_test_server_with_limits(pool, servers, apple_server, max_per_week, 4).await
    }

    async fn create_http_test_server_with_limits(
        pool: PgPool,
        servers: &WiremockServers,
        apple_server: &MockServer,
        max_per_week: u32,
        max_per_year: u32,
    ) -> TestServer {
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
        );
        let config = AppleVerificationConfig {
            allowed_audiences: vec![TEST_APPLE_AUDIENCE.to_string()],
            max_verifications_per_week: max_per_week,
            max_verifications_per_year: max_per_year,
        };
        let db = SqlDb::test(pool).await;
        let hasher = crate::shared::HasherArgon2id::new(
            tempfile::tempdir().unwrap().keep().join("pepper.txt"),
        );
        let apple_verification = AppleVerificationService::for_test(
            db,
            homeserver_api,
            &config,
            hasher,
            jwks_url(apple_server),
        );
        let apple_verification_router = Router::new()
            .route("/", post(root_handler))
            .with_state(AppState { apple_verification });
        let router = Router::new().nest("/apple_verification", apple_verification_router);
        TestServer::new(router).expect("Failed to create test server")
    }

    async fn create_app_test_server(pool: PgPool, config_toml: &str) -> TestServer {
        let config: AppConfig = toml::from_str(config_toml).expect("test config should parse");
        let db = SqlDb::test(pool).await;
        let homeserver_api = HomeserverAdminAPI::new(
            &"http://127.0.0.1:1".parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
        );
        let hasher = crate::shared::HasherArgon2id::new(
            tempfile::tempdir().unwrap().keep().join("pepper.txt"),
        );
        let router = HttpServer::create_router(&config, &db, &homeserver_api, hasher)
            .await
            .expect("router should build");
        TestServer::new(router).expect("Failed to create test server")
    }

    const BASE_CONFIG: &str = r#"
database_url = "postgres://localhost:5432/pubky_homegate"

[homeserver]
admin_api_url = "http://localhost:6288"
admin_password = "test-admin-password"
"#;

    #[sqlx::test]
    async fn http_returns_signup_code_for_success(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let apple_server = jwks_server().await;
        let server = create_http_test_server(pool, &servers, &apple_server, 2).await;

        setup_homeserver_signup_token("token-123")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": valid_token("apple-subject") }))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "token-123");
        assert_eq!(body["homeserverPubky"], "test-homeserver-pubky");
    }

    #[sqlx::test]
    async fn http_maps_invalid_token(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let apple_server = jwks_server().await;
        let server = create_http_test_server(pool, &servers, &apple_server, 2).await;

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": wrong_audience_token() }))
            .await;
        response.assert_status(StatusCode::UNAUTHORIZED);
        assert_eq!(response.text(), "invalid_apple_id_token");
    }

    #[sqlx::test]
    async fn http_maps_apple_verifier_unavailable(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let apple_server = MockServer::start().await;
        let server = create_http_test_server(pool, &servers, &apple_server, 2).await;

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": valid_token("apple-subject") }))
            .await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text(), "apple_verifier_unavailable");
    }

    #[sqlx::test]
    async fn http_maps_homeserver_unavailable(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let apple_server = jwks_server().await;
        let server = create_http_test_server(pool, &servers, &apple_server, 2).await;

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": valid_token("apple-subject") }))
            .await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text(), "homeserver_unavailable");
    }

    #[sqlx::test]
    async fn http_maps_weekly_limit(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let apple_server = jwks_server().await;
        let server = create_http_test_server(pool, &servers, &apple_server, 1).await;
        let token = valid_token("apple-subject");

        setup_homeserver_signup_token("token")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": &token }))
            .await
            .assert_status_ok();

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": &token }))
            .await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.text(), "weekly_limit_exceeded");
    }

    #[sqlx::test]
    async fn http_maps_annual_limit(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let apple_server = jwks_server().await;
        let server =
            create_http_test_server_with_limits(pool.clone(), &servers, &apple_server, 10, 1).await;
        let token = valid_token("apple-subject");

        setup_homeserver_signup_token("token")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": &token }))
            .await
            .assert_status_ok();

        let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
        sqlx::query("UPDATE apple_verifications SET created_at = $1")
            .bind(eight_days_ago)
            .execute(&pool)
            .await
            .expect("Failed to age verification");

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": &token }))
            .await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.text(), "annual_limit_exceeded");
    }

    #[sqlx::test]
    async fn http_route_disabled_without_config_section(pool: PgPool) {
        let server = create_app_test_server(pool, BASE_CONFIG).await;

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": "not-a-jwt" }))
            .await;
        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[sqlx::test]
    async fn http_route_enabled_with_config_section(pool: PgPool) {
        let config = format!(
            "{BASE_CONFIG}\n[apple_verification]\nallowed_audiences = [\"{TEST_APPLE_AUDIENCE}\"]\n"
        );
        let server = create_app_test_server(pool, &config).await;

        let response = server
            .post("/apple_verification")
            .json(&serde_json::json!({ "appleIdToken": "not-a-jwt" }))
            .await;
        response.assert_status(StatusCode::UNAUTHORIZED);
        assert_eq!(response.text(), "invalid_apple_id_token");
    }
}
