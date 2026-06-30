use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};

use crate::infrastructure::{config::GoogleVerificationConfig, http::HttpServerError, sql::SqlDb};
use crate::shared::{HasherArgon2id, HomeserverAdminAPI};

use super::app_state::AppState;
use super::error::GoogleVerificationError;
use super::types::{GoogleVerificationRequest, GoogleVerificationResponse};

const MAX_GOOGLE_VERIFICATION_REQUEST_BYTES: usize = 20 * 1024;

pub async fn router(
    homeserver_api: &HomeserverAdminAPI,
    google: &GoogleVerificationConfig,
    db: SqlDb,
    hasher: HasherArgon2id,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(homeserver_api, google, db, hasher);
    Ok(Router::new()
        .route("/", post(root_handler))
        .with_state(state))
}

async fn root_handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<GoogleVerificationResponse>, GoogleVerificationError> {
    let request = parse_request_body(&body)?;
    let response = state
        .google_verification
        .verify(&request.google_id_token)
        .await?;
    Ok(Json(response))
}

fn parse_request_body(body: &[u8]) -> Result<GoogleVerificationRequest, GoogleVerificationError> {
    if body.len() > MAX_GOOGLE_VERIFICATION_REQUEST_BYTES {
        return Err(GoogleVerificationError::InvalidRequest);
    }

    let request = serde_json::from_slice::<GoogleVerificationRequest>(body)
        .map_err(|_| GoogleVerificationError::InvalidRequest)?;
    if request.google_id_token.trim().is_empty() {
        return Err(GoogleVerificationError::InvalidRequest);
    }

    Ok(request)
}

impl IntoResponse for GoogleVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            GoogleVerificationError::InvalidRequest => StatusCode::BAD_REQUEST,
            GoogleVerificationError::InvalidGoogleIdToken => StatusCode::UNAUTHORIZED,
            GoogleVerificationError::WeeklyLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            GoogleVerificationError::AnnualLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            GoogleVerificationError::HomeserverUnavailable => {
                tracing::error!("Homeserver unavailable during Google verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            GoogleVerificationError::GoogleVerifierUnavailable => {
                tracing::error!("Google verifier unavailable during Google verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            GoogleVerificationError::Database(ref error) => {
                tracing::error!(error = %error, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::routing::post;
    use axum_test::TestServer;
    use sqlx::PgPool;

    use super::*;
    use crate::e2e::{WiremockServers, setup_homeserver_signup_token};
    use crate::google_verification::google_id_token_verifier::{
        GoogleIdTokenVerificationError, GoogleIdTokenVerifier, VerifiedGoogleIdentity,
    };
    use crate::google_verification::service::GoogleVerificationService;
    use crate::infrastructure::config::GoogleVerificationConfig;
    use crate::infrastructure::sql::SqlDb;
    use crate::shared::HomeserverAdminAPI;

    #[test]
    fn test_parse_request_body_rejects_unknown_fields() {
        let err =
            parse_request_body(br#"{"googleIdToken":"token","driveAccessToken":"drive-token"}"#)
                .unwrap_err();
        assert!(matches!(err, GoogleVerificationError::InvalidRequest));
    }

    #[test]
    fn test_parse_request_body_rejects_empty_token() {
        let err = parse_request_body(br#"{"googleIdToken":"   "}"#).unwrap_err();
        assert!(matches!(err, GoogleVerificationError::InvalidRequest));
    }

    #[test]
    fn test_parse_request_body_rejects_oversized_body() {
        let body = format!(
            r#"{{"googleIdToken":"{}"}}"#,
            "a".repeat(MAX_GOOGLE_VERIFICATION_REQUEST_BYTES)
        );

        let err = parse_request_body(body.as_bytes()).unwrap_err();

        assert!(matches!(err, GoogleVerificationError::InvalidRequest));
    }

    async fn create_http_test_server(
        pool: PgPool,
        servers: &WiremockServers,
        verifier: Arc<dyn GoogleIdTokenVerifier>,
        max_per_week: u32,
    ) -> TestServer {
        create_http_test_server_with_limits(pool, servers, verifier, max_per_week, 4).await
    }

    async fn create_http_test_server_with_limits(
        pool: PgPool,
        servers: &WiremockServers,
        verifier: Arc<dyn GoogleIdTokenVerifier>,
        max_per_week: u32,
        max_per_year: u32,
    ) -> TestServer {
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
        );
        let config = GoogleVerificationConfig {
            google_client_id: "test-google-client-id.apps.googleusercontent.com".to_string(),
            max_verifications_per_week: max_per_week,
            max_verifications_per_year: max_per_year,
            signup_quota: None,
        };
        let db = SqlDb::test(pool).await;
        let hasher = crate::shared::HasherArgon2id::new(
            tempfile::tempdir().unwrap().keep().join("pepper.txt"),
        );
        let google_verification =
            GoogleVerificationService::with_verifier(db, homeserver_api, &config, hasher, verifier);
        let google_verification_router =
            Router::new()
                .route("/", post(root_handler))
                .with_state(AppState {
                    google_verification,
                });
        let router = Router::new().nest("/google_verification", google_verification_router);
        TestServer::new(router).expect("Failed to create test server")
    }

    fn fake_valid_verifier() -> Arc<dyn GoogleIdTokenVerifier> {
        Arc::new(FakeGoogleVerifier {
            result: Ok(VerifiedGoogleIdentity {
                issuer: "https://accounts.google.com".to_string(),
                subject: "google-subject".to_string(),
            }),
        })
    }

    fn fake_error_verifier(
        error: GoogleIdTokenVerificationError,
    ) -> Arc<dyn GoogleIdTokenVerifier> {
        Arc::new(FakeGoogleVerifier { result: Err(error) })
    }

    #[derive(Debug)]
    struct FakeGoogleVerifier {
        result: Result<VerifiedGoogleIdentity, GoogleIdTokenVerificationError>,
    }

    #[async_trait]
    impl GoogleIdTokenVerifier for FakeGoogleVerifier {
        async fn verify(
            &self,
            _id_token: &str,
        ) -> Result<VerifiedGoogleIdentity, GoogleIdTokenVerificationError> {
            self.result.clone()
        }
    }

    #[sqlx::test]
    async fn http_returns_signup_code_for_success(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers, fake_valid_verifier(), 2).await;

        setup_homeserver_signup_token("token-123")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "token-123");
        assert_eq!(body["homeserverPubky"], "test-homeserver-pubky");
    }

    #[sqlx::test]
    async fn http_rejects_malformed_request(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers, fake_valid_verifier(), 2).await;

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token", "driveAccessToken": "secret" }))
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
        assert_eq!(response.text(), "invalid_request");
    }

    #[sqlx::test]
    async fn http_maps_invalid_token(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(
            pool,
            &servers,
            fake_error_verifier(GoogleIdTokenVerificationError::Invalid),
            2,
        )
        .await;

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "invalid-token" }))
            .await;
        response.assert_status(StatusCode::UNAUTHORIZED);
        assert_eq!(response.text(), "invalid_google_id_token");
    }

    #[sqlx::test]
    async fn http_maps_google_verifier_unavailable(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(
            pool,
            &servers,
            fake_error_verifier(GoogleIdTokenVerificationError::DependencyUnavailable),
            2,
        )
        .await;

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text(), "google_verifier_unavailable");
    }

    #[sqlx::test]
    async fn http_maps_homeserver_unavailable(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers, fake_valid_verifier(), 2).await;

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text(), "homeserver_unavailable");
    }

    #[sqlx::test]
    async fn http_maps_weekly_limit(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server(pool, &servers, fake_valid_verifier(), 1).await;

        setup_homeserver_signup_token("token")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await
            .assert_status_ok();

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.text(), "weekly_limit_exceeded");
    }

    #[sqlx::test]
    async fn http_maps_annual_limit(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let server = create_http_test_server_with_limits(
            pool.clone(),
            &servers,
            fake_valid_verifier(),
            10,
            1,
        )
        .await;

        setup_homeserver_signup_token("token")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await
            .assert_status_ok();

        let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
        sqlx::query("UPDATE google_verifications SET created_at = $1")
            .bind(eight_days_ago)
            .execute(&pool)
            .await
            .expect("Failed to age verification");

        let response = server
            .post("/google_verification")
            .json(&serde_json::json!({ "googleIdToken": "valid-token" }))
            .await;
        response.assert_status(StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.text(), "annual_limit_exceeded");
    }
}
