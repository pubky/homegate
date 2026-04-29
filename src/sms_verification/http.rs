use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::sms_verification::{
    error::SmsVerificationError,
    types::{CreateVerificationRequest, ValidateCodeRequest, ValidateCodeResponse},
};
use crate::{
    infrastructure::{
        config::SmsVerificationConfig,
        http::{HttpServerError, RequestOrigin, UserAgent},
    },
    shared::HomeserverAdminAPI,
    sms_verification::app_state::AppState,
};

pub async fn router(
    homeserver_api: &HomeserverAdminAPI,
    sms: &SmsVerificationConfig,
    db: crate::infrastructure::sql::SqlDb,
    hasher: crate::shared::HasherArgon2id,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(homeserver_api, sms, db, hasher);
    Ok(Router::new()
        .route("/send_code", post(send_code_handler))
        .route("/validate_code", post(validate_code_handler))
        .route("/info", get(get_info_handler))
        .with_state(state))
}

async fn send_code_handler(
    State(mut state): State<AppState>,
    RequestOrigin(ip_address): RequestOrigin,
    UserAgent(user_agent): UserAgent,
    Json(request): Json<CreateVerificationRequest>,
) -> Result<StatusCode, SmsVerificationError> {
    state
        .sms_verification
        .create_verification(&state.db, request, ip_address, user_agent)
        .await?;
    Ok(StatusCode::OK)
}

async fn validate_code_handler(
    State(mut state): State<AppState>,
    Json(request): Json<ValidateCodeRequest>,
) -> Result<Json<ValidateCodeResponse>, SmsVerificationError> {
    let response = state
        .sms_verification
        .validate_code(&state.db, request)
        .await?;
    Ok(Json(response))
}

/// Get SMS verification info handler.
/// Used by Franky to check if the user is in a country which allows SMS verification.
/// IP-based geo-blocking is done at the nginx level; this endpoint confirms service availability.
async fn get_info_handler() -> StatusCode {
    StatusCode::OK
}

impl IntoResponse for SmsVerificationError {
    fn into_response(self) -> Response {
        let status = match self {
            SmsVerificationError::Blocked => StatusCode::FORBIDDEN,
            SmsVerificationError::WeeklyLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            SmsVerificationError::AnnualLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            SmsVerificationError::MaxValidationAttemptsExceeded => StatusCode::TOO_MANY_REQUESTS,
            SmsVerificationError::NoActiveVerification => StatusCode::UNPROCESSABLE_ENTITY,
            SmsVerificationError::InvalidPhoneNumber => StatusCode::UNPROCESSABLE_ENTITY,
            SmsVerificationError::RateLimited { retry_after } => {
                let mut response =
                    (StatusCode::TOO_MANY_REQUESTS, self.to_string()).into_response();
                if let Some(seconds) = retry_after {
                    response
                        .headers_mut()
                        .insert("Retry-After", seconds.to_string().parse().unwrap());
                }
                return response;
            }
            SmsVerificationError::HomeserverUnavailable => {
                tracing::error!("Homeserver unavailable during SMS verification");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            SmsVerificationError::RequestFailed(ref err) => {
                tracing::error!(error = %err, "Failed to communicate with SMS provider");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            SmsVerificationError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        // If it isnt clear, self.to_string() uses the Display impl for each SmsVerificationError as the Response body
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::{
        WiremockServers, setup_homeserver_signup_token, setup_prelude_check_code,
        setup_prelude_create_verification,
    };
    use crate::infrastructure::config::SmsVerificationConfig;
    use crate::shared::HomeserverAdminAPI;
    use crate::sms_verification::PhoneNumber;
    use axum_test::TestServer;
    use sqlx::PgPool;
    use std::net::SocketAddr;

    // Helper function to create HTTP test server with mocked external APIs
    async fn create_http_test_server(
        pool: PgPool,
        servers: &WiremockServers,
    ) -> (TestServer, PgPool) {
        use crate::infrastructure::sql::SqlDb;

        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
        );
        let sms = SmsVerificationConfig::for_test(servers.prelude_server.uri().parse().unwrap());

        let db = SqlDb::test(pool.clone()).await;

        let hasher = crate::shared::HasherArgon2id::new(
            tempfile::tempdir().unwrap().keep().join("pepper.txt"),
        );
        let sms_verification_router = router(&homeserver_api, &sms, db, hasher)
            .await
            .expect("Failed to create router");

        let router = Router::new().nest("/sms_verification", sms_verification_router);
        let app = router.into_make_service_with_connect_info::<SocketAddr>();
        let server = TestServer::new(app).expect("Failed to create test server");

        (server, pool)
    }

    /// Tests the complete HTTP request/response flow for a successful verification:
    /// - POST /send_code returns 200 with correct JSON response structure
    /// - POST /verify_code returns 200 with success status and signupCode field
    #[sqlx::test]
    async fn http_returns_200_and_correct_json_for_successful_verification(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        let phone = "+30123456789";
        let phone_num = PhoneNumber::new(phone).unwrap();

        setup_prelude_create_verification(&phone_num, None, "success", None)
            .expect(1)
            .mount(&servers.prelude_server)
            .await;

        // Act: POST to /send_code endpoint
        let send_response = server
            .post("/sms_verification/send_code")
            .json(&serde_json::json!({ "phoneNumber": phone }))
            .await;

        // Assert: HTTP 200
        send_response.assert_status_ok();

        // Arrange: Mock successful code verification
        setup_prelude_check_code(&phone_num, "123456", "success")
            .expect(1)
            .mount(&servers.prelude_server)
            .await;

        setup_homeserver_signup_token("token-uuid-123")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        // Act: POST to /verify_code endpoint
        let verify_response: axum_test::TestResponse = server
            .post("/sms_verification/validate_code")
            .json(&serde_json::json!({
                "phoneNumber": phone,
                "code": "123456"
            }))
            .await;

        // Assert: HTTP 200 with valid=true, signupCode, and homeserverPubky in JSON response
        verify_response.assert_status_ok();
        let verify_body: serde_json::Value = verify_response.json();
        assert_eq!(
            verify_body["valid"], "true",
            "Response JSON should have valid=true"
        );
        assert_eq!(
            verify_body["signupCode"], "token-uuid-123",
            "Response JSON should include signupCode"
        );
        assert_eq!(
            verify_body["homeserverPubky"], "test-homeserver-pubky",
            "Response JSON should include homeserverPubky"
        );
    }

    #[sqlx::test]
    async fn test_http_error_response_format(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        // Test invalid phone number returns proper error format
        let response = server
            .post("/sms_verification/send_code")
            .json(&serde_json::json!({ "phoneNumber": "invalid-phone" }))
            .await;

        response.assert_status_unprocessable_entity();

        // Verify the error message contains information about E.164 format
        let body_text = response.text();
        assert!(
            body_text.contains("Invalid phone number format"),
            "Error should mention invalid phone number format"
        );
        assert!(
            body_text.contains("E.164"),
            "Error should reference E.164 format"
        );
    }

    #[sqlx::test]
    async fn test_http_verify_code_status_codes(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;
        let phone = "+30555555555";
        let phone_num = PhoneNumber::new(phone).unwrap();

        // Setup mock for send_code
        setup_prelude_create_verification(&phone_num, None, "success", None)
            .expect(1)
            .mount(&servers.prelude_server)
            .await;

        // First send a code
        server
            .post("/sms_verification/send_code")
            .json(&serde_json::json!({ "phoneNumber": phone }))
            .await;

        // Setup mock for wrong code - returns "failure"
        setup_prelude_check_code(&phone_num, "111111", "failure")
            .expect(1)
            .mount(&servers.prelude_server)
            .await;

        // Test wrong code returns success status but failure in response
        let response = server
            .post("/sms_verification/validate_code")
            .json(&serde_json::json!({
                "phoneNumber": phone,
                "code": "111111"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(
            body["valid"], "false",
            "Response should have valid=false for invalid code"
        );

        // Setup mock for correct code - returns "success"
        setup_prelude_check_code(&phone_num, "123456", "success")
            .expect(1)
            .mount(&servers.prelude_server)
            .await;

        // Setup homeserver signup token mock
        setup_homeserver_signup_token("token-uuid-456")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        // Test correct code returns success
        let response = server
            .post("/sms_verification/validate_code")
            .json(&serde_json::json!({
                "phoneNumber": phone,
                "code": "123456"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["valid"], "true", "Response should have valid=true");
        assert_eq!(body["signupCode"], "token-uuid-456");
        assert_eq!(
            body["homeserverPubky"], "test-homeserver-pubky",
            "Response should include homeserverPubky"
        );

        // Wiremock verifies all expected calls were made
    }

    /// Tests the /info endpoint returns 200 OK.
    #[sqlx::test]
    async fn test_info_endpoint_returns_ok(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        let response = server.get("/sms_verification/info").await;
        response.assert_status_ok();
    }
}
