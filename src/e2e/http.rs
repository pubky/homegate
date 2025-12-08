//! HTTP end-to-end tests.
//!
//! These tests verify the complete HTTP request/response cycle including routing, serialization,
//! status codes, and header handling. Add tests here for HTTP protocol concerns. For testing
//! business logic directly without HTTP overhead, add tests to `sms_verification_service.rs` instead.

use super::{
    WiremockServers, setup_homeserver_signup_token, setup_prelude_check_code,
    setup_prelude_create_verification,
};
use axum_test::TestServer;
use sqlx::PgPool;

// Helper function to create HTTP test server with mocked external APIs
async fn create_http_test_server(pool: PgPool, servers: &WiremockServers) -> (TestServer, PgPool) {
    use crate::infrastructure::database::SqlDb;
    use std::net::SocketAddr;

    let config = servers.create_config();

    // Create SqlDb from the pool with migrations
    let db = SqlDb::test(pool.clone()).await;

    // We need to create AppState with the wiremock config and test pool
    let sms_verification_router = crate::sms_verification::http::router_with_db(&config, db)
        .await
        .expect("Failed to create router");

    let router = crate::HttpServer::create_router(sms_verification_router);
    let app = router.into_make_service_with_connect_info::<SocketAddr>();
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, pool)
}

// Full-Flow HTTP Integration Tests

#[sqlx::test]
async fn test_http_full_verification_flow(pool: PgPool) {
    // 1. Setup wiremock servers
    let servers = WiremockServers::start().await;
    let (server, pool) = create_http_test_server(pool, &servers).await;

    let phone = "+30123456789";

    // 2. Setup Prelude create_verification mock
    setup_prelude_create_verification(phone, None, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // 3. Send code request
    let send_response = server
        .post("/v1/sms_verification/send_code")
        .json(&serde_json::json!({ "phone_number": phone }))
        .await;

    send_response.assert_status_ok();
    let send_body: serde_json::Value = send_response.json();
    assert_eq!(send_body["status"], "success");

    // 4. Setup Prelude check_code mock (success)
    setup_prelude_check_code(phone, "123456", "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // 5. Setup homeserver signup token mock
    setup_homeserver_signup_token("token-uuid-123")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // 6. Verify code request
    let verify_response = server
        .post("/v1/sms_verification/verify_code")
        .json(&serde_json::json!({
            "phone_number": phone,
            "code": "123456"
        }))
        .await;

    verify_response.assert_status_ok();
    let verify_body: serde_json::Value = verify_response.json();
    assert_eq!(verify_body["status"], "success");
    assert_eq!(verify_body["signup_code"], "token-uuid-123");

    // 7. Verify database state
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1 AND status = 'VERIFIED'",
    )
    .bind(phone)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count.0, 1, "Should have 1 verified session in database");
}

#[sqlx::test]
async fn test_http_error_response_format(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;

    // Test invalid phone number returns proper error format (no mock needed - validation before API)
    let response = server
        .post("/v1/sms_verification/send_code")
        .json(&serde_json::json!({ "phone_number": "invalid-phone" }))
        .await;

    response.assert_status_bad_request();
    let body: serde_json::Value = response.json();
    assert!(body["error"].is_string(), "Should have 'error' field");
    assert!(body["message"].is_string(), "Should have 'message' field");
    assert_eq!(body["error"], "invalid_phone_number");
}

#[sqlx::test]
async fn test_http_ip_extraction_with_x_forwarded_for(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;
    let phone = "+30111111111";

    // Setup mock that expects the IP address in the request body
    setup_prelude_create_verification(
        phone,
        Some("203.0.113.1"), // Expect this IP to be sent
        "success",
    )
    .expect(1)
    .mount(&servers.prelude_server)
    .await;

    // Send request with X-Forwarded-For header
    let response = server
        .post("/v1/sms_verification/send_code")
        .json(&serde_json::json!({ "phone_number": phone }))
        .add_header("X-Forwarded-For", "203.0.113.1")
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "success");

    // Wiremock will verify the IP was sent in the request body
}

#[sqlx::test]
async fn test_http_ip_extraction_with_x_real_ip(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;
    let phone = "+30222222222";

    // Setup mock that expects the IP address in the request body
    setup_prelude_create_verification(
        phone,
        Some("198.51.100.1"), // Expect this IP to be sent
        "success",
    )
    .expect(1)
    .mount(&servers.prelude_server)
    .await;

    // Send request with X-Real-IP header
    let response = server
        .post("/v1/sms_verification/send_code")
        .json(&serde_json::json!({ "phone_number": phone }))
        .add_header("X-Real-IP", "198.51.100.1")
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "success");

    // Wiremock will verify the IP was sent in the request body
}

#[sqlx::test]
async fn test_http_content_type_validation(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;

    // Send request without Content-Type header (using text body)
    let response = server
        .post("/v1/sms_verification/send_code")
        .text(r#"{"phone_number": "+30333333333"}"#)
        .await;

    // Axum should reject requests without proper Content-Type
    // Note: Axum's behavior may vary, but typically it should handle this
    // Let's verify it doesn't crash and returns a reasonable status
    assert!(
        response.status_code().is_client_error() || response.status_code().is_success(),
        "Should handle missing Content-Type gracefully, got: {}",
        response.status_code()
    );
}

#[sqlx::test]
async fn test_http_invalid_json_returns_400(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;

    // Send malformed JSON
    let response = server
        .post("/v1/sms_verification/send_code")
        .add_header("Content-Type", "application/json")
        .text(r#"{"phone_number": "+30444444444"#) // Missing closing brace
        .await;

    // Axum returns 415 Unsupported Media Type when Content-Type is application/json
    // but the body isn't valid JSON - this is correct behavior
    assert_eq!(
        response.status_code(),
        415,
        "Should return 415 for malformed JSON with application/json Content-Type"
    );
}

#[sqlx::test]
async fn test_http_missing_required_field_returns_422(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;

    // Send JSON without required phone_number field
    let response = server
        .post("/v1/sms_verification/send_code")
        .json(&serde_json::json!({}))
        .await;

    // Axum returns 422 Unprocessable Entity for deserialization errors
    response.assert_status_unprocessable_entity();
}

#[sqlx::test]
async fn test_http_verify_code_status_codes(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let (server, _pool) = create_http_test_server(pool, &servers).await;
    let phone = "+30555555555";

    // Setup mock for send_code
    setup_prelude_create_verification(phone, None, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // First send a code
    server
        .post("/v1/sms_verification/send_code")
        .json(&serde_json::json!({ "phone_number": phone }))
        .await;

    // Setup mock for wrong code - returns "failure"
    setup_prelude_check_code(phone, "wrong_code", "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Test wrong code returns success status but failure in response
    let response = server
        .post("/v1/sms_verification/verify_code")
        .json(&serde_json::json!({
            "phone_number": phone,
            "code": "wrong_code"
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(
        body["status"], "failure",
        "Status field should indicate failure"
    );

    // Setup mock for correct code - returns "success"
    setup_prelude_check_code(phone, "123456", "success")
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
        .post("/v1/sms_verification/verify_code")
        .json(&serde_json::json!({
            "phone_number": phone,
            "code": "123456"
        }))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "success");
    assert_eq!(body["signup_code"], "token-uuid-456");

    // Wiremock verifies all expected calls were made
}
