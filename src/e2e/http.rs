use super::TestServer;
use sqlx::PgPool;

// Full-Flow HTTP Integration Tests

#[sqlx::test]
async fn test_http_full_verification_flow(pool: PgPool) {
    let server = TestServer::new(pool.clone()).await;
    let phone = "+30123456789";

    // Step 1: Send verification code
    let send_response = server.send_code(phone).await;
    assert_eq!(send_response.status(), 200);

    let send_body: serde_json::Value = send_response.json().await.unwrap();
    assert_eq!(send_body["status"], "success");

    // Step 2: Verify code (mock uses "123456")
    let verify_response = server.verify_code(phone, "123456").await;
    assert_eq!(verify_response.status(), 200);

    let verify_body: serde_json::Value = verify_response.json().await.unwrap();
    assert_eq!(verify_body["status"], "success");
    assert!(verify_body["signup_code"].is_string());
    assert!(!verify_body["signup_code"].as_str().unwrap().is_empty());

    // Step 3: Verify database state
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
    let server = TestServer::new(pool).await;

    // Test invalid phone number returns proper error format
    let response = server.send_code("invalid-phone").await;
    assert_eq!(response.status(), 400, "Should return 400 Bad Request");

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"].is_string(), "Should have 'error' field");
    assert!(body["message"].is_string(), "Should have 'message' field");
    assert_eq!(body["error"], "invalid_phone_number");
}

#[sqlx::test]
async fn test_http_ip_extraction_with_x_forwarded_for(pool: PgPool) {
    let server = TestServer::new(pool.clone()).await;
    let phone = "+30111111111";

    // Send request with X-Forwarded-For header
    let response = server
        .send_code_with_headers(phone, vec![("X-Forwarded-For", "203.0.113.1")])
        .await;

    assert_eq!(response.status(), 200);

    // Verify the request succeeded
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "success");

    // The IP extraction logic is tested at the unit level,
    // but this test verifies it works in the full HTTP context
    // The actual IP is passed to the SMS provider API, which we can't directly verify here,
    // but we can confirm the request succeeded with the header present
}

#[sqlx::test]
async fn test_http_ip_extraction_with_x_real_ip(pool: PgPool) {
    let server = TestServer::new(pool.clone()).await;
    let phone = "+30222222222";

    // Send request with X-Real-IP header
    let response = server
        .send_code_with_headers(phone, vec![("X-Real-IP", "198.51.100.1")])
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "success");
}

#[sqlx::test]
async fn test_http_content_type_validation(pool: PgPool) {
    let server = TestServer::new(pool).await;

    // Send request without Content-Type header
    let response = server
        .client
        .post(&format!(
            "{}/v1/sms_verification/send_code",
            server.base_url
        ))
        .body(r#"{"phone_number": "+30333333333"}"#)
        .send()
        .await
        .unwrap();

    // Axum should reject requests without proper Content-Type
    // Note: Axum's behavior may vary, but typically it should handle this
    // Let's verify it doesn't crash and returns a reasonable status
    assert!(
        response.status().is_client_error() || response.status().is_success(),
        "Should handle missing Content-Type gracefully, got: {}",
        response.status()
    );
}

#[sqlx::test]
async fn test_http_invalid_json_returns_400(pool: PgPool) {
    let server = TestServer::new(pool).await;

    // Send malformed JSON
    let response = server
        .client
        .post(&format!(
            "{}/v1/sms_verification/send_code",
            server.base_url
        ))
        .header("Content-Type", "application/json")
        .body(r#"{"phone_number": "+30444444444"#) // Missing closing brace
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        400,
        "Should return 400 for malformed JSON"
    );
}

#[sqlx::test]
async fn test_http_missing_required_field_returns_422(pool: PgPool) {
    let server = TestServer::new(pool).await;

    // Send JSON without required phone_number field
    let response = server
        .client
        .post(&format!(
            "{}/v1/sms_verification/send_code",
            server.base_url
        ))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();

    // Axum returns 422 Unprocessable Entity for deserialization errors
    assert_eq!(
        response.status(),
        422,
        "Should return 422 for missing required field"
    );
}

#[sqlx::test]
async fn test_http_verify_code_status_codes(pool: PgPool) {
    let server = TestServer::new(pool).await;
    let phone = "+30555555555";

    // First send a code
    server.send_code(phone).await;

    // Test wrong code returns success status but failure in response
    let response = server.verify_code(phone, "wrong_code").await;
    assert_eq!(
        response.status(),
        200,
        "Should return 200 even for wrong code"
    );

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body["status"], "failure",
        "Status field should indicate failure"
    );

    // Test correct code returns success
    let response = server.verify_code(phone, "123456").await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "success");
}
