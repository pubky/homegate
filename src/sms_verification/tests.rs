//! Service-layer integration tests for SMS verification.
//!
//! These tests verify business logic by calling service methods directly with real database
//! and mocked external APIs (Prelude, Homeserver). Add tests here for business rules, state
//! transitions, and edge cases. For HTTP-specific concerns (status codes, headers, JSON parsing),
//! add tests to `http.rs` instead.

use crate::e2e::{
    WiremockServers, setup_homeserver_signup_token, setup_prelude_check_code,
    setup_prelude_create_verification,
};
use crate::infrastructure::sql::{DbError, SqlDb};
use crate::shared::HomeserverAdminAPI;
use crate::sms_verification::error::SmsVerificationError;
use crate::sms_verification::hasher_argon2id::HasherArgon2id;
use crate::sms_verification::prelude_api::{PreludeAPI, PreludeBlockedReason};
use crate::sms_verification::repository::{SmsVerificationRepository, VerificationStatus};
use crate::sms_verification::service::SmsVerificationService;
use crate::sms_verification::{
    Code, CreateVerificationRequest, PhoneNumber, ValidateCodeRequest, ValidateCodeResponse,
};
use sqlx::PgPool;
use std::net::IpAddr;

const TEST_VERIFICATION_CODE: &str = "123456";
const TEST_WRONG_CODE: &str = "111111";
// TODO replace with faster hasher
fn test_phone_hasher() -> HasherArgon2id {
    HasherArgon2id::new()
}

/// Helper to create service with wiremock for direct service layer testing
fn create_service_with_mocked_apis(servers: &WiremockServers) -> SmsVerificationService {
    use crate::EnvConfig;

    let config = EnvConfig::for_test(
        servers.prelude_server.uri().parse().unwrap(),
        servers.homeserver_server.uri().parse().unwrap(),
    );

    let prelude_api = PreludeAPI::new(
        config.prelude_api_url.as_ref().unwrap(),
        &config.prelude_api_key,
    );
    let homeserver_admin_api = HomeserverAdminAPI::new(
        &config.homeserver_admin_api_url,
        &config.homeserver_admin_password,
        &config.homeserver_pubky,
    );
    SmsVerificationService::new(prelude_api, homeserver_admin_api, 2, 4, 2, vec![])
}

#[sqlx::test]
async fn test_service_full_verification_flow(pool: PgPool) {
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, ResponseTemplate};

    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30123456789").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());
    let user_agent = Some("Mozilla/5.0 (Test Client)".to_string());
    let dispatch_id = Some("test-dispatch-id-123".to_string());

    // Setup wiremock expectations - verify exact request body with signals
    let expected_body = json!({
        "target": {
            "type": "phone_number",
            "value": phone.as_str()
        },
        "signals": {
            "ip": ip.map(|ip| ip.to_string()).unwrap(),
            "user_agent": user_agent.clone().unwrap()
        },
        "dispatch_id": dispatch_id.clone().unwrap()
    });

    Mock::given(method("POST"))
        .and(path("/v2/verification"))
        .and(header("Authorization", "Bearer test-key"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(&expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "verification-id-123",
            "status": "success"
        })))
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-123")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // Step 1: Initiate verification with user_agent and dispatch_id
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: dispatch_id.clone(),
            },
            ip,
            user_agent.clone(),
        )
        .await
        .expect("verify_init should succeed");

    // Step 1.5: Check database after initiation
    let mut executor = db.pool().into();
    let after_init = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .expect("Should find verification after init");

    let hashed_phone = test_phone_hasher().hash(phone.as_str());
    assert_eq!(
        after_init.phone_number_hash, hashed_phone,
        "Phone number should be hashed"
    );
    let verification_id = after_init.prelude_id.clone();
    assert!(
        after_init.finalised_at.is_none(),
        "finalised_at should be NULL after init"
    );
    assert!(
        after_init.signup_code.is_none(),
        "signup_code should be NULL after init"
    );
    assert_eq!(
        after_init.status,
        VerificationStatus::Pending,
        "status should be PENDING after init"
    );

    // Step 2: Verify code
    let check_response = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("verify_finalise should succeed");

    assert!(matches!(check_response, ValidateCodeResponse::Valid { .. }));

    // Step 3: Query database to verify state updated correctly
    let after_verify =
        SmsVerificationRepository::get_by_prelude_id(&mut executor, &verification_id)
            .await
            .expect("Should find verification in database");

    assert_eq!(
        after_verify.phone_number_hash, hashed_phone,
        "Phone number should still be hashed"
    );
    assert!(
        after_verify.finalised_at.is_some(),
        "finalised_at should be set after successful verification"
    );

    // Check that finalised_at is recent (within last minute)
    let finalised_at = after_verify.finalised_at.unwrap();
    let now = chrono::Utc::now().naive_utc();
    let diff_secs = (now.and_utc().timestamp() - finalised_at.and_utc().timestamp()).abs();
    assert!(
        diff_secs < 60,
        "finalised_at should be recent (was {} seconds ago)",
        diff_secs
    );

    // Check signup code
    assert!(
        after_verify.signup_code.is_some(),
        "signup_code should be generated after verification"
    );
    let signup_code_str = after_verify.signup_code.as_ref().unwrap();
    assert_eq!(
        after_verify.status,
        VerificationStatus::Verified,
        "status should be VERIFIED after successful verification"
    );
    assert!(
        !signup_code_str.is_empty(),
        "signup_code should not be empty"
    );
}

#[sqlx::test]
async fn test_service_session_lifecycle(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    // Test 1: Active session reuse
    let phone1 = PhoneNumber::new("+30999999999").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Setup mock - we can use "success" for both calls
    // The important thing is that the DB correctly handles session reuse
    setup_prelude_create_verification(&phone1, ip, "success", None)
        .expect(2) // Both calls will use this mock
        .mount(&servers.prelude_server)
        .await;

    // First send_code creates active session
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone1.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("First send_code should succeed");

    let hashed_phone1 = test_phone_hasher().hash(phone1.as_str());
    let count1: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone1)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count1.0, 1, "Should have 1 active session");

    // Second send_code - API might return success again, but our code should
    // see the existing pending session in DB and not create a duplicate
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone1.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("Second send_code should succeed");

    // Key assertion: DB should still have only 1 record (no duplicate created)
    let count2: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone1)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count2.0, 1, "Should still have only 1 record (reused)");

    // Test 2: New session after verification
    let phone2 = PhoneNumber::new("+30000000000").unwrap();

    // Setup mocks for full verification flow + retry after verification
    // We'll call send_code twice (once before verification, once after)
    setup_prelude_create_verification(&phone2, ip, "success", None)
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone2, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone2.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("send_code should succeed");

    service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone2.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("verify_code should succeed");

    // After verification, new send_code creates a new session
    // Mock already set up above with expect(2)
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone2.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("send_code after verification should succeed");

    let hashed_phone2 = test_phone_hasher().hash(phone2.as_str());
    let count3: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone2)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(
        count3.0, 2,
        "Should have 2 records (1 verified, 1 new active)"
    );
}

#[sqlx::test]
async fn test_service_max_verified_sessions_limits(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;
    let phone = PhoneNumber::new("+30111111112").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Setup mocks for successful verifications (4 complete verifications)
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(4)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(4)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(4)
        .mount(&servers.homeserver_server)
        .await;

    // Test weekly limit: Complete 2 verifications (within last 7 days)
    for i in 0..2 {
        service
            .create_verification(
                &db,
                CreateVerificationRequest {
                    phone_number: phone.clone(),
                    dispatch_id: None,
                },
                ip,
                None,
            )
            .await
            .unwrap_or_else(|_| panic!("send_code {} should succeed", i));

        service
            .validate_code(
                &db,
                ValidateCodeRequest {
                    phone_number: phone.clone(),
                    code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
                },
            )
            .await
            .unwrap_or_else(|_| panic!("verify_code {} should succeed", i));
    }

    // 3rd attempt should fail weekly limit (no mock needed - validation happens before API call)
    let result = service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await;

    assert!(result.is_err(), "3rd send_code should fail");
    match result {
        Err(SmsVerificationError::WeeklyLimitExceeded) => {}
        _ => panic!("Expected WeeklyLimitExceeded error"),
    }

    // Age one verification to 8 days ago (outside weekly window)
    let hashed_phone = test_phone_hasher().hash(phone.as_str());
    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query(
        "UPDATE sms_verifications
         SET finalised_at = $1
         WHERE id = (
             SELECT id FROM sms_verifications
             WHERE phone_number_hash = $2
             AND status = 'VERIFIED'
             ORDER BY finalised_at ASC
             LIMIT 1
         )",
    )
    .bind(eight_days_ago)
    .bind(&hashed_phone)
    .execute(db.pool())
    .await
    .expect("Failed to age verification");

    // Now 3rd attempt should succeed (1 within weekly window, 1 aged out)
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("3rd send_code should succeed after aging");

    service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("3rd verify_code should succeed");

    // Now we have: 1 aged out (8 days), 2 within weekly window
    // We need to complete 2 more verifications (4th and 5th) to reach annual limit
    // But first age another one so we can do both without hitting weekly limit

    // Age the second verification to 8 days ago
    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query(
        "UPDATE sms_verifications
         SET finalised_at = $1
         WHERE id = (
             SELECT id FROM sms_verifications
             WHERE phone_number_hash = $2
             AND status = 'VERIFIED'
             AND finalised_at > $1
             ORDER BY finalised_at ASC
             LIMIT 1
         )",
    )
    .bind(eight_days_ago)
    .bind(&hashed_phone)
    .execute(db.pool())
    .await
    .expect("Failed to age second verification");

    // Now we have: 2 aged out, 1 within weekly window
    // Complete 4th verification
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("4th send_code should succeed");

    service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("4th verify_code should succeed");

    // Now we have: 2 aged out, 2 within weekly window (3rd and 4th)
    // Age the 3rd one so we can complete the 5th without hitting weekly limit
    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query(
        "UPDATE sms_verifications
         SET finalised_at = $1
         WHERE id = (
             SELECT id FROM sms_verifications
             WHERE phone_number_hash = $2
             AND status = 'VERIFIED'
             AND finalised_at > $1
             ORDER BY finalised_at ASC
             LIMIT 1
         )",
    )
    .bind(eight_days_ago)
    .bind(&hashed_phone)
    .execute(db.pool())
    .await
    .expect("Failed to age third verification");

    // 5th attempt should fail annual limit (we have 4 total, all verified)
    let result = service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await;

    assert!(result.is_err(), "5th send_code should fail");
    match result {
        Err(SmsVerificationError::AnnualLimitExceeded) => {}
        other => panic!("Expected AnnualLimitExceeded error, got: {:?}", other),
    }
}

/// Tests that whitelisted phone numbers bypass verification rate limits.
/// The whitelist is configured via SMS_VERIFICATIONS_LIMIT_WHITELIST env var.
#[sqlx::test]
async fn test_service_whitelist_bypasses_limits(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let phone = PhoneNumber::new("+30111111113").unwrap();

    // Create service and add this phone number to the whitelist
    let mut service = create_service_with_mocked_apis(&servers);
    service.set_limit_whitelist(vec![phone.clone()]);
    let db = SqlDb::test(pool.clone()).await;
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Setup mocks for many successful verifications (more than weekly + annual limits combined)
    // Weekly limit is 2, annual limit is 4, so we'll do 5 to prove whitelist bypasses both
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(5)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(5)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(5)
        .mount(&servers.homeserver_server)
        .await;

    // Complete 5 verifications - all should succeed due to whitelist
    for i in 0..5 {
        service
            .create_verification(
                &db,
                CreateVerificationRequest {
                    phone_number: phone.clone(),
                    dispatch_id: None,
                },
                ip,
                None,
            )
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "send_code {} should succeed for whitelisted number, got: {:?}",
                    i, e
                )
            });

        service
            .validate_code(
                &db,
                ValidateCodeRequest {
                    phone_number: phone.clone(),
                    code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
                },
            )
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "verify_code {} should succeed for whitelisted number, got: {:?}",
                    i, e
                )
            });
    }

    // Verify all 5 verifications are in the database as VERIFIED
    let hashed_phone = test_phone_hasher().hash(phone.as_str());
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1 AND status = 'VERIFIED'",
    )
    .bind(&hashed_phone)
    .fetch_one(db.pool())
    .await
    .unwrap();

    assert_eq!(
        count.0, 5,
        "All 5 verifications should be VERIFIED for whitelisted number"
    );
}

/// Tests that non-whitelisted phone numbers are still subject to rate limits
/// when a whitelist is configured (whitelist doesn't disable limits globally).
#[sqlx::test]
async fn test_service_whitelist_does_not_affect_other_numbers(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let whitelisted_phone = PhoneNumber::new("+30111111114").unwrap();
    let regular_phone = PhoneNumber::new("+30111111115").unwrap();

    // Create service and add only whitelisted_phone to the whitelist
    let mut service = create_service_with_mocked_apis(&servers);
    service.set_limit_whitelist(vec![whitelisted_phone]);
    let db = SqlDb::test(pool.clone()).await;
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Setup mocks for regular phone (2 successful verifications)
    setup_prelude_create_verification(&regular_phone, ip, "success", None)
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&regular_phone, TEST_VERIFICATION_CODE, "success")
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    // Complete 2 verifications for regular phone (reaching weekly limit)
    for i in 0..2 {
        service
            .create_verification(
                &db,
                CreateVerificationRequest {
                    phone_number: regular_phone.clone(),
                    dispatch_id: None,
                },
                ip,
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("send_code {} should succeed, got: {:?}", i, e));

        service
            .validate_code(
                &db,
                ValidateCodeRequest {
                    phone_number: regular_phone.clone(),
                    code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("verify_code {} should succeed, got: {:?}", i, e));
    }

    // 3rd attempt for regular phone should fail (weekly limit)
    let result = service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: regular_phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await;

    match result {
        Err(SmsVerificationError::WeeklyLimitExceeded) => {
            // Test passes - regular phone is still rate limited
        }
        other => panic!(
            "Expected WeeklyLimitExceeded for non-whitelisted number, got: {:?}",
            other
        ),
    }
}

#[sqlx::test]
async fn test_service_input_validation_and_errors(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Invalid verification code - should return Failure status
    let phone_wrong_code = PhoneNumber::new("+30987654321").unwrap();

    setup_prelude_create_verification(&phone_wrong_code, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone_wrong_code, TEST_WRONG_CODE, "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone_wrong_code.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("send_code should succeed");

    let check_response = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone_wrong_code.clone(),
                code: Code::new(TEST_WRONG_CODE).unwrap(),
            },
        )
        .await
        .expect("API should respond");

    assert!(
        matches!(check_response, ValidateCodeResponse::Invalid),
        "Wrong code should return Invalid"
    );

    // Verify database NOT updated when wrong code provided
    let hashed_phone_wrong = test_phone_hasher().hash(phone_wrong_code.as_str());
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1 AND status = 'VERIFIED'",
    )
    .bind(&hashed_phone_wrong)
    .fetch_one(db.pool())
    .await
    .unwrap();

    assert_eq!(count.0, 0, "Should not mark as verified with wrong code");
}

#[sqlx::test]
async fn test_service_expired_or_not_found_marks_failed(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30666666666").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Step 1: Create a verification session
    // We'll call send_code twice (once here, once after marking failed), so expect(2)
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("send_code should succeed");

    // Step 2: Verify initial status
    let hashed_phone_expired = test_phone_hasher().hash(phone.as_str());
    let status_before: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_expired)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification record");

    assert_eq!(status_before, "PENDING", "Should start as PENDING");

    // Step 3: Mock Prelude check_code to return expired_or_not_found
    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "expired_or_not_found")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Step 4: Try to verify with code - this should trigger mark_failed
    let verify_result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Verify the response is NoActiveVerification error
    match verify_result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - correct error type
        }
        other => panic!("Expected NoActiveVerification error, got: {:?}", other),
    }

    // Step 5: Verify database state
    let mut executor = db.pool().into();
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .expect("Should find verification record");

    assert_eq!(
        record.status,
        VerificationStatus::Failed,
        "status should be FAILED"
    );
    assert_eq!(
        record.failure_reason,
        Some("expired_or_not_found".to_string()),
        "failure_reason should be set"
    );
    assert!(record.finalised_at.is_some(), "finalised_at should be set");

    // Step 6: Verify that check_pending_exists returns NotFound
    let result = SmsVerificationRepository::err_if_no_active_verification(
        &mut executor,
        &hashed_phone_expired,
    )
    .await;
    assert!(
        matches!(result, Err(DbError::NotFound(_))),
        "Failed sessions should not be considered active"
    );

    // Step 7: Verify that we can create a new session after failure
    // Mock already set up above with expect(2)
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("Should be able to create new session after failure");

    // Verify we now have 2 records (1 FAILED, 1 PENDING)
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_expired)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count.0, 2, "Should have 2 records after retry");

    let pending_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1 AND status = 'PENDING'",
    )
    .bind(&hashed_phone_expired)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(pending_count.0, 1, "Should have 1 PENDING record");
}

#[sqlx::test]
async fn test_service_success_but_homeserver_fails_marks_failed(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30888888888").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Create verification session
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Mock successful code check from Prelude
    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Mock homeserver to return error when generating signup token
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("GET"))
        .and(path("/generate_signup_token"))
        .and(header("X-Admin-Password", "test-pass"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // Validate code - should fail due to homeserver error
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    assert!(result.is_err(), "Should fail due to homeserver error");

    // Verify session marked FAILED (not stuck in PENDING)
    let mut executor = db.pool().into();
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .unwrap();
    assert_eq!(record.status, VerificationStatus::Failed);
    assert_eq!(
        record.failure_reason,
        Some("homeserver_signup_token_generation_failed".to_string())
    );

    // Verify that failed verification does NOT count towards quota
    let phone_hash = test_phone_hasher().hash(phone.as_str());
    let failed_count = SmsVerificationRepository::count_verified_sessions_in_last_days(
        &mut executor,
        &phone_hash,
        7,
    )
    .await
    .unwrap();
    assert_eq!(
        failed_count, 0,
        "Failed verification should NOT count towards weekly quota"
    );
}

// This circumstance happens if validate_code() is called before send_code() - Prelude has no prelude_id for the verification session yet so it returns a dummy value which doesnt match to anything in our db.
#[sqlx::test]
async fn test_service_expired_or_not_found_with_mismatched_prelude_id(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30777777777").unwrap();
    let hashed_phone = test_phone_hasher().hash(phone.as_str());
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Create verification session with prelude_id "verification-id-123"
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Verify initial PENDING status
    let mut executor = db.pool().into();
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .unwrap();
    assert_eq!(record.status, VerificationStatus::Pending);
    assert_eq!(record.prelude_id, "verification-id-123");

    // Mock Prelude to return expired_or_not_found with DIFFERENT prelude_id
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("POST"))
        .and(path("/v2/verification/check"))
        .and(header("Authorization", "Bearer test-key"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(json!({
            "target": {
                "type": "phone_number",
                "value": phone.as_str()
            },
            "code": TEST_VERIFICATION_CODE
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "session-99",  // Different ID that doesn't exist in our DB
            "status": "expired_or_not_found",
            "metadata": null,
            "request_id": null
        })))
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Validate code - should still mark session as failed via phone fallback
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Should return NoActiveVerification error
    assert!(matches!(
        result,
        Err(SmsVerificationError::NoActiveVerification)
    ));

    // Verify session marked FAILED despite prelude_id mismatch
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .unwrap();
    assert_eq!(record.status, VerificationStatus::Failed);
    assert_eq!(
        record.failure_reason,
        Some("expired_or_not_found".to_string())
    );
    assert!(record.finalised_at.is_some());

    // Verify no active verification remains
    let result =
        SmsVerificationRepository::err_if_no_active_verification(&mut executor, &hashed_phone)
            .await;
    assert!(matches!(result, Err(DbError::NotFound(_))));
}

#[sqlx::test]
async fn test_repository_mark_failed_by_phone_number(pool: PgPool) {
    let db = SqlDb::test(pool.clone()).await;
    let mut executor = db.pool().into();

    let phone = PhoneNumber::new("+30999999999").unwrap();
    let hashed_phone = test_phone_hasher().hash(phone.as_str());

    // Create PENDING session
    SmsVerificationRepository::create_verification(&mut executor, &hashed_phone, "test-prelude-id")
        .await
        .unwrap();

    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .unwrap();
    assert_eq!(record.status, VerificationStatus::Pending);

    // Mark failed by phone number
    SmsVerificationRepository::mark_all_pending_verification_as_failed(
        &mut executor,
        &hashed_phone,
        "test_reason",
    )
    .await
    .unwrap();

    // Verify updated state
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .unwrap();
    assert_eq!(record.status, VerificationStatus::Failed);
    assert_eq!(record.failure_reason, Some("test_reason".to_string()));
    assert!(record.finalised_at.is_some());

    // Test idempotency - should return NotFound (no PENDING sessions)
    let result = SmsVerificationRepository::mark_all_pending_verification_as_failed(
        &mut executor,
        &hashed_phone,
        "another_reason",
    )
    .await;
    assert!(matches!(result, Err(DbError::NotFound(_))));
}

#[sqlx::test]
async fn test_service_verify_code_with_wrong_phone_number(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone_send = PhoneNumber::new("+30666666666").unwrap();
    let phone_verify = PhoneNumber::new("+30888888889").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Step 1: Send verification code to phone_send
    setup_prelude_create_verification(&phone_send, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone_send.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("send_code should succeed");

    // Step 2: Try to verify with a different phone number (no mock needed - DB lookup fails first)
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone_verify.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Should fail with NoActiveVerification error since there's no pending verification for phone_verify
    match result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - correct error type
        }
        other => panic!("Expected NoActiveVerification, got: {:?}", other),
    }

    // Step 3: Verify that the original phone number still has a pending verification
    let mut executor = db.pool().into();
    let hashed_phone_send = test_phone_hasher().hash(phone_send.as_str());
    let check_result =
        SmsVerificationRepository::err_if_no_active_verification(&mut executor, &hashed_phone_send)
            .await;
    assert!(
        check_result.is_ok(),
        "Original phone number should still have pending verification"
    );
}

#[sqlx::test]
async fn test_service_database_error_handling(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30777777777").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // First, create a session with send_code
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("send_code should succeed");

    // Now manually delete the database record to simulate database inconsistency
    let hashed_phone_db_error = test_phone_hasher().hash(phone.as_str());
    sqlx::query("DELETE FROM sms_verifications WHERE phone_number_hash = $1")
        .bind(&hashed_phone_db_error)
        .execute(db.pool())
        .await
        .expect("Failed to delete record");

    // Setup mock - it will succeed but database lookup will fail
    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(0) // Should not be called - DB lookup fails first
        .mount(&servers.prelude_server)
        .await;

    // Now try to verify - database lookup fails before API call
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Should propagate NoActiveVerification error from database through service layer
    match result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - database error was properly propagated
        }
        other => panic!("Expected NoActiveVerification, got: {:?}", other),
    }
}

#[sqlx::test]
async fn test_service_verify_code_on_terminal_states(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Scenario 1: Attempt send_code on VERIFIED state
    let phone_verified = PhoneNumber::new("+30111111111").unwrap();

    // Setup mocks for successful verification flow
    setup_prelude_create_verification(&phone_verified, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone_verified, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-verified")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // Create and verify successfully
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone_verified.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone_verified.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("send_code should succeed");

    // Verify state is VERIFIED
    let hashed_phone_verified = test_phone_hasher().hash(phone_verified.as_str());
    let status: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_verified)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");
    assert_eq!(status, "VERIFIED", "Status should be VERIFIED");

    // Attempt to send code again on VERIFIED state - should fail
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone_verified.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Should return NoActiveVerification error
    match result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected NoActiveVerification on VERIFIED state, got: {:?}",
            other
        ),
    }

    // Verify database state unchanged
    let status_after: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_verified)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");
    assert_eq!(
        status_after, "VERIFIED",
        "Status should remain VERIFIED after failed attempt"
    );

    // Scenario 2: Attempt send_code on FAILED state
    let phone_failed = PhoneNumber::new("+30222222222").unwrap();

    // Create verification
    setup_prelude_create_verification(&phone_failed, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone_failed.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Mock expired_or_not_found to reach FAILED state
    setup_prelude_check_code(
        &phone_failed,
        TEST_VERIFICATION_CODE,
        "expired_or_not_found",
    )
    .expect(1)
    .mount(&servers.prelude_server)
    .await;

    let verify_result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone_failed.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Verify the response is NoActiveVerification error
    match verify_result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - correct error type
        }
        other => panic!("Expected NoActiveVerification error, got: {:?}", other),
    }

    // Verify state is FAILED
    let hashed_phone_failed = test_phone_hasher().hash(phone_failed.as_str());
    let status_failed: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_failed)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");
    assert_eq!(status_failed, "FAILED", "Status should be FAILED");

    // Attempt to send code again on FAILED state - should fail
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone_failed.clone(),
                code: Code::new(TEST_WRONG_CODE).unwrap(),
            },
        )
        .await;

    // Should return NoActiveVerification error
    match result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected NoActiveVerification on FAILED state, got: {:?}",
            other
        ),
    }

    // Verify database state unchanged
    let status_after_failed: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_failed)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");
    assert_eq!(
        status_after_failed, "FAILED",
        "Status should remain FAILED after failed attempt"
    );
}

#[sqlx::test]
async fn test_service_multiple_wrong_code_attempts(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30333333333").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Create verification
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Setup mock for 1 wrong code attempt (max_failed_validation_attempts is 2 in tests,
    // so we can only do 1 wrong attempt before the correct one on attempt 2)
    setup_prelude_check_code(&phone, TEST_WRONG_CODE, "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Attempt 1: Wrong code
    let response1 = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_WRONG_CODE).unwrap(),
            },
        )
        .await
        .expect("send_code should succeed");

    assert!(
        matches!(response1, ValidateCodeResponse::Invalid),
        "Wrong code should return Invalid"
    );

    // Verify status remains PENDING
    let mut executor = db.pool().into();
    let record1 = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .expect("Should find verification");

    assert_eq!(
        record1.status,
        VerificationStatus::Pending,
        "Status should remain PENDING"
    );
    assert!(
        record1.finalised_at.is_none(),
        "finalised_at should remain NULL after wrong code"
    );
    assert_eq!(record1.attempts, 1, "Should have 1 attempt recorded");

    // Finally: Correct code
    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-correct")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    let response_success = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("send_code with correct code should succeed");

    assert!(
        matches!(response_success, ValidateCodeResponse::Valid { .. }),
        "Correct code should return Valid"
    );

    // Verify final state is VERIFIED
    let hashed_phone_multi = test_phone_hasher().hash(phone.as_str());
    let (status_final, finalised_at_final, signup_code): (
        String,
        Option<chrono::NaiveDateTime>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT status, finalised_at, signup_code FROM sms_verifications WHERE phone_number_hash = $1",
    )
    .bind(&hashed_phone_multi)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification");

    assert_eq!(status_final, "VERIFIED", "Status should be VERIFIED");
    assert!(
        finalised_at_final.is_some(),
        "finalised_at should be set after verification"
    );
    assert!(
        signup_code.is_some(),
        "signup_code should be set after verification"
    );
}

#[sqlx::test]
async fn test_service_blocked_phone_number(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30444444444").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Mock Prelude to return "blocked" status with repeated_attempts reason
    setup_prelude_create_verification(
        &phone,
        ip,
        "blocked",
        Some(PreludeBlockedReason::RepeatedAttempts),
    )
    .expect(1)
    .mount(&servers.prelude_server)
    .await;

    // Call create_verification
    let response = service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await;

    // Verify the response is NoActiveVerification error
    match response {
        Err(SmsVerificationError::Blocked) => {
            // Test passes - correct error type
        }
        other => panic!("Expected Blocked error, got: {:?}", other),
    }

    // Verify database state: record should be created and marked as FAILED
    let hashed_phone_blocked = test_phone_hasher().hash(phone.as_str());
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_blocked)
            .fetch_one(db.pool())
            .await
            .unwrap();

    assert_eq!(
        count.0, 1,
        "Record should be created even for blocked response"
    );

    let (status, failure_reason, finalised_at): (
        String,
        Option<String>,
        Option<chrono::NaiveDateTime>,
    ) = sqlx::query_as(
        "SELECT status, failure_reason, finalised_at FROM sms_verifications WHERE phone_number_hash = $1",
    )
    .bind(&hashed_phone_blocked)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification");

    assert_eq!(
        status, "FAILED",
        "Status should be FAILED for blocked phone"
    );
    assert!(
        failure_reason.is_some(),
        "failure_reason should be set for blocked phone"
    );
    let failure_reason = failure_reason.unwrap();
    assert!(
        failure_reason.contains("repeated_attempts"),
        "failure_reason should contain the blocked reason, got: {}",
        failure_reason
    );
    assert!(
        finalised_at.is_some(),
        "finalised_at should be set for blocked phone"
    );

    // Verify that check_pending_exists returns NotFound
    let mut executor = db.pool().into();
    let result = SmsVerificationRepository::err_if_no_active_verification(
        &mut executor,
        &hashed_phone_blocked,
    )
    .await;
    assert!(
        matches!(result, Err(DbError::NotFound(_))),
        "Blocked (FAILED) sessions should not be considered active"
    );
}

#[sqlx::test]
async fn test_service_retry_response_from_prelude(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30555555555").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Mock Prelude to return "retry" status (rate limit response)
    setup_prelude_create_verification(&phone, ip, "retry", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Call create_verification - should get Retry response from Prelude
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Verify that a PENDING session was created (even for retry response)
    // Based on service.rs:88-94, retry responses still create a record
    let hashed_phone_retry = test_phone_hasher().hash(phone.as_str());
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_retry)
            .fetch_one(db.pool())
            .await
            .unwrap();

    assert_eq!(
        count.0, 1,
        "Should have created 1 PENDING session even for retry response"
    );

    let status: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_retry)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");

    assert_eq!(
        status, "PENDING",
        "Status should be PENDING for retry response"
    );
}

#[sqlx::test]
async fn test_repository_state_mutation_protection(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Scenario 1: mark_verified on already VERIFIED record
    let phone1 = PhoneNumber::new("+30666666666").unwrap();

    setup_prelude_create_verification(&phone1, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone1, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-1")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // Create and verify successfully
    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone1.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone1.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("send_code should succeed");

    // Get the prelude_id and original signup_code
    let mut executor = db.pool().into();
    let record1 = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone1)
        .await
        .expect("Should find verification");
    let prelude_id1 = record1.prelude_id;
    let original_signup_code = record1.signup_code.expect("signup_code should be set");

    // Try to call mark_verified again with different signup code
    let different_signup_code = "different-signup-code";
    let result = SmsVerificationRepository::mark_verified(
        &mut executor,
        &prelude_id1,
        different_signup_code,
    )
    .await;

    // Should return NotFound error (0 rows affected)
    match result {
        Err(DbError::NotFound(_)) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected NotFound error on VERIFIED record, got: {:?}",
            other
        ),
    }

    // Verify database unchanged (still has original signup code)
    let signup_code_after: String =
        sqlx::query_scalar("SELECT signup_code FROM sms_verifications WHERE prelude_id = $1")
            .bind(&prelude_id1)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");

    assert_eq!(
        signup_code_after, original_signup_code,
        "Signup code should be unchanged"
    );

    // Scenario 2: mark_failed on already VERIFIED record
    let result =
        SmsVerificationRepository::mark_failed(&mut executor, &prelude_id1, "test_reason").await;

    // Should return NotFound error (0 rows affected)
    match result {
        Err(DbError::NotFound(_)) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected NotFound error when marking VERIFIED as failed, got: {:?}",
            other
        ),
    }

    // Verify database unchanged (still VERIFIED)
    let status1: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE prelude_id = $1")
            .bind(&prelude_id1)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");

    assert_eq!(
        status1, "VERIFIED",
        "Status should remain VERIFIED after failed mark_failed attempt"
    );

    // Scenario 3: mark_verified on already FAILED record
    // Use a different phone number than phone1
    let phone2 = PhoneNumber::new("+30777777777").unwrap();

    // Note: wiremock will return the same prelude_id for both phone1 and phone2,
    // but since they have different phone_numbers, they are separate records in the database
    setup_prelude_create_verification(&phone2, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone2.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Mock expired_or_not_found to reach FAILED state
    setup_prelude_check_code(&phone2, TEST_VERIFICATION_CODE, "expired_or_not_found")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    let verify_result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone2.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    // Should return NoActiveVerification error
    match verify_result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - correct error type
        }
        other => panic!("Expected NoActiveVerification error, got: {:?}", other),
    }

    // Verify we have a FAILED record before trying to mark it as verified
    let hashed_phone2_mut = test_phone_hasher().hash(phone2.as_str());
    let (prelude_id2, status_before_mark): (String, String) = sqlx::query_as(
        "SELECT prelude_id, status FROM sms_verifications WHERE phone_number_hash = $1",
    )
    .bind(&hashed_phone2_mut)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification");

    assert_eq!(
        status_before_mark, "FAILED",
        "Status should be FAILED before attempting mark_verified"
    );

    // Try to call mark_verified on FAILED record
    let signup_code2 = "another-signup-code";
    let result =
        SmsVerificationRepository::mark_verified(&mut executor, &prelude_id2, signup_code2).await;

    // Should return NotFound error (0 rows affected)
    match result {
        Err(DbError::NotFound(_)) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected NotFound error when marking FAILED as verified, got: {:?}",
            other
        ),
    }

    // Verify database unchanged (still FAILED) - query by phone_number_hash to avoid ambiguity
    let status2: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone2_mut)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");

    assert_eq!(
        status2, "FAILED",
        "Status should remain FAILED after failed mark_verified attempt"
    );

    // Verify signup_code is still NULL for FAILED record
    let signup_code_failed: Option<String> = sqlx::query_scalar(
        "SELECT signup_code FROM sms_verifications WHERE phone_number_hash = $1",
    )
    .bind(&hashed_phone2_mut)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification");

    assert!(
        signup_code_failed.is_none(),
        "Signup code should remain NULL for FAILED record"
    );
}

#[sqlx::test]
async fn test_create_verification_session_supersession(pool: PgPool) {
    let db = SqlDb::test(pool.clone()).await;
    let mut executor = db.pool().into();

    // Scenario 1: Different prelude_id supersedes existing PENDING session
    let phone1 = PhoneNumber::new("+30555555551").unwrap();
    let hashed_phone1 = test_phone_hasher().hash(phone1.as_str());
    let prelude_id_1 = "prelude-id-1";
    let prelude_id_2 = "prelude-id-2";

    SmsVerificationRepository::create_verification(&mut executor, &hashed_phone1, prelude_id_1)
        .await
        .expect("First create should succeed");

    let record1 = SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_1)
        .await
        .expect("Should find first session");
    assert_eq!(record1.status, VerificationStatus::Pending);

    // Create second session with different prelude_id - should supersede first
    SmsVerificationRepository::create_verification(&mut executor, &hashed_phone1, prelude_id_2)
        .await
        .expect("Second create should succeed");

    let old_record = SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_1)
        .await
        .expect("Should find old session");
    assert_eq!(old_record.status, VerificationStatus::Failed);
    assert_eq!(
        old_record.failure_reason,
        Some("superseded_by_new_session".to_string())
    );
    assert!(old_record.finalised_at.is_some());

    let new_record = SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_2)
        .await
        .expect("Should find new session");
    assert_eq!(new_record.status, VerificationStatus::Pending);

    let count1: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone1)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count1.0, 2, "Should have 2 records after supersession");

    // Scenario 2: Same prelude_id is idempotent (no duplicate created)
    let phone2 = PhoneNumber::new("+30555555552").unwrap();
    let hashed_phone2 = test_phone_hasher().hash(phone2.as_str());
    let prelude_id_same = "prelude-id-same";

    SmsVerificationRepository::create_verification(&mut executor, &hashed_phone2, prelude_id_same)
        .await
        .expect("First create should succeed");

    let before_retry = SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_same)
        .await
        .expect("Should find session");
    assert_eq!(before_retry.status, VerificationStatus::Pending);

    // Retry with same prelude_id - should be idempotent
    SmsVerificationRepository::create_verification(&mut executor, &hashed_phone2, prelude_id_same)
        .await
        .expect("Retry should succeed (idempotent)");

    let count2: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone2)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count2.0, 1, "Should have only 1 record (idempotent)");

    let after_retry = SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_same)
        .await
        .expect("Should find session");
    assert_eq!(after_retry.status, VerificationStatus::Pending);

    // Scenario 3: New session allowed after FAILED session
    let phone3 = PhoneNumber::new("+30555555553").unwrap();
    let hashed_phone3 = test_phone_hasher().hash(phone3.as_str());
    let prelude_id_failed = "prelude-id-failed";
    let prelude_id_after_failed = "prelude-id-after-failed";

    SmsVerificationRepository::create_verification(
        &mut executor,
        &hashed_phone3,
        prelude_id_failed,
    )
    .await
    .expect("Create should succeed");

    SmsVerificationRepository::mark_failed(&mut executor, prelude_id_failed, "test_failure")
        .await
        .expect("Mark failed should succeed");

    let failed_record =
        SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_failed)
            .await
            .expect("Should find failed session");
    assert_eq!(failed_record.status, VerificationStatus::Failed);

    // Create new session - should succeed without superseding FAILED session
    SmsVerificationRepository::create_verification(
        &mut executor,
        &hashed_phone3,
        prelude_id_after_failed,
    )
    .await
    .expect("Create after failed should succeed");

    let still_failed =
        SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_failed)
            .await
            .expect("Should find old failed session");
    assert_eq!(still_failed.status, VerificationStatus::Failed);
    assert_eq!(
        still_failed.failure_reason,
        Some("test_failure".to_string())
    );

    let new_pending =
        SmsVerificationRepository::get_by_prelude_id(&mut executor, prelude_id_after_failed)
            .await
            .expect("Should find new session");
    assert_eq!(new_pending.status, VerificationStatus::Pending);

    let count3: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone3)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count3.0, 2, "Should have 2 records (1 failed, 1 pending)");
}

/// Tests that exceeding max failed validation attempts marks session as failed and returns error.
/// The default max_failed_validation_attempts in tests is 2.
#[sqlx::test]
async fn test_service_max_validation_attempts_exceeded(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30123123123").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Create verification session
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Setup mock for wrong code attempts - Prelude returns "failure" for wrong codes
    // We'll make 2 wrong attempts (the limit), then the 3rd should fail with MaxValidationAttemptsExceeded
    setup_prelude_check_code(&phone, TEST_WRONG_CODE, "failure")
        .expect(2) // Only 2 calls to Prelude - 3rd attempt fails before API call
        .mount(&servers.prelude_server)
        .await;

    // Attempts 1-2: Wrong code, should return Invalid
    for i in 1..=2 {
        let response = service
            .validate_code(
                &db,
                ValidateCodeRequest {
                    phone_number: phone.clone(),
                    code: Code::new(TEST_WRONG_CODE).unwrap(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("Attempt {} should succeed, got error: {:?}", i, e));

        assert!(
            matches!(response, ValidateCodeResponse::Invalid),
            "Attempt {} should return Invalid",
            i
        );
    }

    // Verify attempts counter is at 2
    let mut executor = db.pool().into();
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .expect("Should find verification");
    assert_eq!(record.attempts, 2, "Should have 2 attempts recorded");
    assert_eq!(
        record.status,
        VerificationStatus::Pending,
        "Should still be PENDING after 2 attempts"
    );

    // Attempt 3: Should fail with MaxValidationAttemptsExceeded (no API call made)
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_WRONG_CODE).unwrap(),
            },
        )
        .await;

    match result {
        Err(SmsVerificationError::MaxValidationAttemptsExceeded) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected MaxValidationAttemptsExceeded error on 3rd attempt, got: {:?}",
            other
        ),
    }

    // Verify session is now FAILED
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .expect("Should find verification");
    assert_eq!(
        record.status,
        VerificationStatus::Failed,
        "Should be FAILED after exceeding max attempts"
    );
    assert_eq!(
        record.failure_reason,
        Some("max_validation_attempts_exceeded".to_string()),
        "failure_reason should indicate max attempts exceeded"
    );
    assert!(
        record.finalised_at.is_some(),
        "finalised_at should be set after failure"
    );
    assert_eq!(
        record.attempts, 3,
        "Should have 3 attempts recorded (including the failed one)"
    );

    // Subsequent attempts should return NoActiveVerification (session is FAILED)
    let result = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await;

    match result {
        Err(SmsVerificationError::NoActiveVerification) => {
            // Test passes - no active session after failure
        }
        other => panic!(
            "Expected NoActiveVerification after session failed, got: {:?}",
            other
        ),
    }
}

/// Tests that a correct code on the last allowed attempt still succeeds.
#[sqlx::test]
async fn test_service_correct_code_on_last_attempt_succeeds(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let mut service = create_service_with_mocked_apis(&servers);
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30456456456").unwrap();
    let ip: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());

    // Create verification session
    setup_prelude_create_verification(&phone, ip, "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            &db,
            CreateVerificationRequest {
                phone_number: phone.clone(),
                dispatch_id: None,
            },
            ip,
            None,
        )
        .await
        .expect("create_verification should succeed");

    // Setup mocks: 1 wrong attempt, then 1 correct on the 2nd (last allowed) attempt
    setup_prelude_check_code(&phone, TEST_WRONG_CODE, "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, TEST_VERIFICATION_CODE, "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-last-chance")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // Attempt 1: Wrong code
    let response = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_WRONG_CODE).unwrap(),
            },
        )
        .await
        .expect("Attempt 1 should succeed");

    assert!(
        matches!(response, ValidateCodeResponse::Invalid),
        "Attempt 1 should return Invalid"
    );

    // Attempt 2 (last chance): Correct code should succeed
    let response = service
        .validate_code(
            &db,
            ValidateCodeRequest {
                phone_number: phone.clone(),
                code: Code::new(TEST_VERIFICATION_CODE).unwrap(),
            },
        )
        .await
        .expect("2nd attempt with correct code should succeed");

    assert!(
        matches!(response, ValidateCodeResponse::Valid { .. }),
        "Correct code on last attempt should return Valid"
    );

    // Verify session is VERIFIED
    let mut executor = db.pool().into();
    let record = SmsVerificationRepository::get_by_phone_number(&mut executor, &phone)
        .await
        .expect("Should find verification");
    assert_eq!(
        record.status,
        VerificationStatus::Verified,
        "Should be VERIFIED after correct code"
    );
    assert_eq!(record.attempts, 2, "Should have 2 attempts recorded");
}
