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
use crate::infrastructure::database::{DbError, SqlDb};
use crate::sms_verification::hasher_argon2id::HasherArgon2id;
use crate::sms_verification::prelude_api::PreludeAPI;
use crate::sms_verification::repository::{SmsVerificationRepository, VerificationStatus};
use crate::sms_verification::{
    CreateVerificationRequest, CreateVerificationResponse, PhoneNumber, SendCodeRequest,
    SendCodeResponse,
};
use crate::{HomeserverAdminAPI, SmsVerificationService};
use sqlx::PgPool;
use std::net::IpAddr;

fn test_phone_hasher() -> HasherArgon2id {
    HasherArgon2id::new("test-pepper-for-phone-number-hashing".to_string())
}

/// Helper to create service with wiremock for direct service layer testing
async fn create_service_with_mocked_apis(
    pool: PgPool,
    servers: &WiremockServers,
) -> SmsVerificationService {
    use crate::EnvConfig;

    let config = EnvConfig::for_test(
        servers.prelude_server.uri().parse().unwrap(),
        servers.homeserver_server.uri().parse().unwrap(),
    );
    let db = SqlDb::test(pool.clone()).await;

    let prelude_api = PreludeAPI::new(&config.prelude_api_url, &config.prelude_api_key);
    let homeserver_admin_api = HomeserverAdminAPI::new(
        &config.homeserver_admin_api_url,
        &config.homeserver_admin_password,
        &config.homeserver_pubky,
    );

    let phone_hasher = HasherArgon2id::new(config.phone_number_pepper.clone());
    let repository = SmsVerificationRepository::new(db, phone_hasher);
    SmsVerificationService::new(repository, prelude_api, homeserver_admin_api, 10)
}

#[sqlx::test]
async fn test_service_full_verification_flow(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30123456789").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Setup wiremock expectations
    setup_prelude_create_verification(&phone, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, "123456", "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-123")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    // Step 1: Initiate verification
    let verify_response = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await
        .expect("verify_init should succeed");

    assert!(matches!(
        verify_response,
        CreateVerificationResponse::Success
    ));

    // Step 1.5: Check database after initiation
    let repository = SmsVerificationRepository::new(db.clone(), test_phone_hasher());
    let after_init = repository
        .get_by_phone_number(&phone)
        .await
        .expect("Should find verification after init");

    let hashed_phone = test_phone_hasher()
        .hash_phone_number(phone.as_str())
        .unwrap();
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
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("verify_finalise should succeed");

    assert!(matches!(check_response, SendCodeResponse::Success { .. }));

    // Step 3: Query database to verify state updated correctly
    let after_verify = repository
        .get_by_prelude_id(&verification_id)
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
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    // Test 1: Active session reuse
    let phone1 = PhoneNumber::new("+30999999999").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Setup mock - we can use "success" for both calls
    // The important thing is that the DB correctly handles session reuse
    setup_prelude_create_verification(&phone1, Some(ip), "success", None)
        .expect(2) // Both calls will use this mock
        .mount(&servers.prelude_server)
        .await;

    // First send_code creates active session
    let response1 = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone1.clone(),
            },
            ip,
        )
        .await
        .expect("First send_code should succeed");

    assert!(matches!(response1, CreateVerificationResponse::Success));

    let hashed_phone1 = test_phone_hasher()
        .hash_phone_number(phone1.as_str())
        .unwrap();
    let count1: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone1)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count1.0, 1, "Should have 1 active session");

    // Second send_code - API might return success again, but our code should
    // see the existing pending session in DB and not create a duplicate
    let _response2 = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone1.clone(),
            },
            ip,
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
    setup_prelude_create_verification(&phone2, Some(ip), "success", None)
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone2, "123456", "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone2.clone(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    service
        .send_code(SendCodeRequest {
            phone_number: phone2.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("verify_code should succeed");

    // After verification, new send_code creates a new session
    // Mock already set up above with expect(2)
    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone2.clone(),
            },
            ip,
        )
        .await
        .expect("send_code after verification should succeed");

    let hashed_phone2 = test_phone_hasher()
        .hash_phone_number(phone2.as_str())
        .unwrap();
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
async fn test_service_max_verified_sessions_limit(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let phone = PhoneNumber::new("+30111111112").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Setup mocks for 10 successful verifications
    setup_prelude_create_verification(&phone, Some(ip), "success", None)
        .expect(10)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, "123456", "success")
        .expect(10)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(10)
        .mount(&servers.homeserver_server)
        .await;

    // Complete 10 verifications
    for i in 0..10 {
        service
            .create_verification(
                CreateVerificationRequest {
                    phone_number: phone.clone(),
                },
                ip,
            )
            .await
            .expect(&format!("send_code {} should succeed", i));

        service
            .send_code(SendCodeRequest {
                phone_number: phone.clone(),
                code: "123456".to_string(),
            })
            .await
            .expect(&format!("verify_code {} should succeed", i));
    }

    // 11th attempt should fail (no mock needed - validation happens before API call)
    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await;

    assert!(result.is_err(), "11th send_code should fail");
    match result {
        Err(crate::SmsVerificationError::TooManyVerifiedSessions) => {}
        _ => panic!("Expected TooManyVerifiedSessions error"),
    }
}

#[sqlx::test]
async fn test_service_input_validation_and_errors(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Invalid verification code - should return Failure status
    let phone_wrong_code = PhoneNumber::new("+30987654321").unwrap();

    setup_prelude_create_verification(&phone_wrong_code, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone_wrong_code, "wrong_code", "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone_wrong_code.clone(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    let check_response = service
        .send_code(SendCodeRequest {
            phone_number: phone_wrong_code.clone(),
            code: "wrong_code".to_string(),
        })
        .await
        .expect("API should respond");

    assert!(
        matches!(check_response, SendCodeResponse::Failure),
        "Wrong code should return Failure status"
    );

    // Verify database NOT updated when wrong code provided
    let hashed_phone_wrong = test_phone_hasher()
        .hash_phone_number(phone_wrong_code.as_str())
        .unwrap();
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number_hash = $1 AND status = 'VERIFIED'",
    )
    .bind(&hashed_phone_wrong)
    .fetch_one(db.pool())
    .await
    .unwrap();

    assert_eq!(count.0, 0, "Should not mark as verified with wrong code");

    // Boundary: 9 verified sessions should allow 10th
    let phone = PhoneNumber::new("+30888888888").unwrap();

    // Setup mocks for 10 verifications (9 + 1 more)
    setup_prelude_create_verification(&phone, Some(ip), "success", None)
        .expect(10)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone, "123456", "success")
        .expect(10)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token")
        .expect(10)
        .mount(&servers.homeserver_server)
        .await;

    // Complete 9 verifications
    for i in 0..9 {
        service
            .create_verification(
                CreateVerificationRequest {
                    phone_number: phone.clone(),
                },
                ip,
            )
            .await
            .expect(&format!("send_code {} should succeed", i));
        service
            .send_code(SendCodeRequest {
                phone_number: phone.clone(),
                code: "123456".to_string(),
            })
            .await
            .expect(&format!("verify_code {} should succeed", i));
    }

    // 10th should succeed

    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await;
    assert!(result.is_ok(), "10th verification should succeed");

    // Complete the 10th verification (mocks already set up above with expect(10))
    service
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("verify_code for 10th session should succeed");

    // At limit: 10 verified sessions should reject 11th (no mock needed)
    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await;
    assert!(
        matches!(
            result,
            Err(crate::SmsVerificationError::TooManyVerifiedSessions)
        ),
        "11th send_code should fail with TooManyVerifiedSessions"
    );
}

#[sqlx::test]
async fn test_service_expired_or_not_found_marks_failed(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30666666666").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Step 1: Create a verification session
    // We'll call send_code twice (once here, once after marking failed), so expect(2)
    setup_prelude_create_verification(&phone, Some(ip), "success", None)
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    // Step 2: Verify initial status
    let hashed_phone_expired = test_phone_hasher()
        .hash_phone_number(phone.as_str())
        .unwrap();
    let status_before: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_expired)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification record");

    assert_eq!(status_before, "PENDING", "Should start as PENDING");

    // Step 3: Mock Prelude check_code to return expired_or_not_found
    setup_prelude_check_code(&phone, "123456", "expired_or_not_found")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Step 4: Try to verify with code - this should trigger mark_failed
    let verify_result = service
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("send_code should succeed");

    // Verify the response is ExpiredOrNotFound
    assert!(
        matches!(verify_result, SendCodeResponse::ExpiredOrNotFound),
        "Should return ExpiredOrNotFound"
    );

    // Step 5: Verify database state
    let repository = SmsVerificationRepository::new(db.clone(), test_phone_hasher());
    let record = repository
        .get_by_phone_number(&phone)
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
    let result = repository.err_if_no_active_verification(&phone).await;
    assert!(
        matches!(result, Err(DbError::NotFound(_))),
        "Failed sessions should not be considered active"
    );

    // Step 7: Verify that we can create a new session after failure
    // Mock already set up above with expect(2)
    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
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
async fn test_service_verify_code_with_wrong_phone_number(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone_send = PhoneNumber::new("+30666666666").unwrap();
    let phone_verify = PhoneNumber::new("+30888888889").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Step 1: Send verification code to phone_send
    setup_prelude_create_verification(&phone_send, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone_send.clone(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    // Step 2: Try to verify with a different phone number (no mock needed - DB lookup fails first)
    let result = service
        .send_code(SendCodeRequest {
            phone_number: phone_verify.clone(),
            code: "123456".to_string(),
        })
        .await;

    // Should fail with NoActiveVerification error since there's no pending verification for phone_verify
    match result {
        Err(crate::SmsVerificationError::NoActiveVerification(_)) => {
            // Test passes - correct error type
        }
        other => panic!("Expected NoActiveVerification, got: {:?}", other),
    }

    // Step 3: Verify that the original phone number still has a pending verification
    let repository = SmsVerificationRepository::new(db.clone(), test_phone_hasher());
    let check_result = repository.err_if_no_active_verification(&phone_send).await;
    assert!(
        check_result.is_ok(),
        "Original phone number should still have pending verification"
    );
}

#[sqlx::test]
async fn test_service_database_error_handling(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30777777777").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // First, create a session with send_code
    setup_prelude_create_verification(&phone, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    // Now manually delete the database record to simulate database inconsistency
    let hashed_phone_db_error = test_phone_hasher()
        .hash_phone_number(phone.as_str())
        .unwrap();
    sqlx::query("DELETE FROM sms_verifications WHERE phone_number_hash = $1")
        .bind(&hashed_phone_db_error)
        .execute(db.pool())
        .await
        .expect("Failed to delete record");

    // Setup mock - it will succeed but database lookup will fail
    setup_prelude_check_code(&phone, "123456", "success")
        .expect(0) // Should not be called - DB lookup fails first
        .mount(&servers.prelude_server)
        .await;

    // Now try to verify - database lookup fails before API call
    let result = service
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "123456".to_string(),
        })
        .await;

    // Should propagate NoActiveVerification error from database through service layer
    match result {
        Err(crate::SmsVerificationError::NoActiveVerification(_)) => {
            // Test passes - database error was properly propagated
        }
        other => panic!("Expected NoActiveVerification, got: {:?}", other),
    }
}

#[sqlx::test]
async fn test_service_verify_code_on_terminal_states(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Scenario 1: Attempt send_code on VERIFIED state
    let phone_verified = PhoneNumber::new("+30111111111").unwrap();

    // Setup mocks for successful verification flow
    setup_prelude_create_verification(&phone_verified, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone_verified, "123456", "success")
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
            CreateVerificationRequest {
                phone_number: phone_verified.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    service
        .send_code(SendCodeRequest {
            phone_number: phone_verified.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("send_code should succeed");

    // Verify state is VERIFIED
    let hashed_phone_verified = test_phone_hasher()
        .hash_phone_number(phone_verified.as_str())
        .unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_verified)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");
    assert_eq!(status, "VERIFIED", "Status should be VERIFIED");

    // Attempt to send code again on VERIFIED state - should fail
    let result = service
        .send_code(SendCodeRequest {
            phone_number: phone_verified.clone(),
            code: "123456".to_string(),
        })
        .await;

    // Should return NoActiveVerification error
    match result {
        Err(crate::SmsVerificationError::NoActiveVerification(_)) => {
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
    setup_prelude_create_verification(&phone_failed, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone_failed.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    // Mock expired_or_not_found to reach FAILED state
    setup_prelude_check_code(&phone_failed, "123456", "expired_or_not_found")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    let verify_result = service
        .send_code(SendCodeRequest {
            phone_number: phone_failed.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("send_code should succeed");

    assert!(
        matches!(verify_result, SendCodeResponse::ExpiredOrNotFound),
        "Should return ExpiredOrNotFound"
    );

    // Verify state is FAILED
    let hashed_phone_failed = test_phone_hasher()
        .hash_phone_number(phone_failed.as_str())
        .unwrap();
    let status_failed: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number_hash = $1")
            .bind(&hashed_phone_failed)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification");
    assert_eq!(status_failed, "FAILED", "Status should be FAILED");

    // Attempt to send code again on FAILED state - should fail
    let result = service
        .send_code(SendCodeRequest {
            phone_number: phone_failed.clone(),
            code: "999999".to_string(),
        })
        .await;

    // Should return NoActiveVerification error
    match result {
        Err(crate::SmsVerificationError::NoActiveVerification(_)) => {
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
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30333333333").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Create verification
    setup_prelude_create_verification(&phone, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    // Attempt 1: Wrong code
    setup_prelude_check_code(&phone, "111111", "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    let response1 = service
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "111111".to_string(),
        })
        .await
        .expect("send_code should succeed");

    assert!(
        matches!(response1, SendCodeResponse::Failure),
        "Wrong code should return Failure"
    );

    // Verify status remains PENDING
    let repository = SmsVerificationRepository::new(db.clone(), test_phone_hasher());
    let record1 = repository
        .get_by_phone_number(&phone)
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

    // Attempt 2: Wrong code
    setup_prelude_check_code(&phone, "222222", "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    let response2 = service
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "222222".to_string(),
        })
        .await
        .expect("send_code should succeed");

    assert!(
        matches!(response2, SendCodeResponse::Failure),
        "Wrong code should return Failure"
    );

    // Verify status still PENDING
    let record2 = repository
        .get_by_phone_number(&phone)
        .await
        .expect("Should find verification");

    assert_eq!(
        record2.status,
        VerificationStatus::Pending,
        "Status should still be PENDING"
    );
    assert!(
        record2.finalised_at.is_none(),
        "finalised_at should remain NULL after second wrong code"
    );

    // Finally: Correct code
    setup_prelude_check_code(&phone, "123456", "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_homeserver_signup_token("test-token-correct")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    let response_success = service
        .send_code(SendCodeRequest {
            phone_number: phone.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("send_code with correct code should succeed");

    assert!(
        matches!(response_success, SendCodeResponse::Success { .. }),
        "Correct code should return Success"
    );

    // Verify final state is VERIFIED
    let hashed_phone_multi = test_phone_hasher()
        .hash_phone_number(phone.as_str())
        .unwrap();
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
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30444444444").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Mock Prelude to return "blocked" status with repeated_attempts reason
    setup_prelude_create_verification(
        &phone,
        Some(ip),
        "blocked",
        Some(PreludeBlockedReason::RepeatedAttempts),
    )
    .expect(1)
    .mount(&servers.prelude_server)
    .await;

    // Call create_verification
    let response = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    // Assert response is Blocked with correct reason
    use crate::sms_verification::prelude_api::PreludeBlockedReason;
    match response {
        CreateVerificationResponse::Blocked { reason } => {
            assert_eq!(
                reason,
                PreludeBlockedReason::RepeatedAttempts,
                "Blocked reason should be RepeatedAttempts"
            );
        }
        other => panic!("Expected Blocked response, got: {:?}", other),
    }

    // Verify database state: record should be created and marked as FAILED
    let hashed_phone_blocked = test_phone_hasher()
        .hash_phone_number(phone.as_str())
        .unwrap();
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
        failure_reason.contains("RepeatedAttempts"),
        "failure_reason should contain the blocked reason, got: {}",
        failure_reason
    );
    assert!(
        finalised_at.is_some(),
        "finalised_at should be set for blocked phone"
    );

    // Verify that check_pending_exists returns NotFound
    let repository = SmsVerificationRepository::new(db.clone(), test_phone_hasher());
    let result = repository.err_if_no_active_verification(&phone).await;
    assert!(
        matches!(result, Err(DbError::NotFound(_))),
        "Blocked (FAILED) sessions should not be considered active"
    );
}

#[sqlx::test]
async fn test_service_retry_response_from_prelude(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = PhoneNumber::new("+30555555555").unwrap();
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Mock Prelude to return "retry" status (rate limit response)
    setup_prelude_create_verification(&phone, Some(ip), "retry", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Call create_verification - should get Retry response from Prelude
    let response = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    // Assert response is Retry
    match response {
        CreateVerificationResponse::Retry => {
            // Test passes
        }
        other => panic!("Expected Retry response, got: {:?}", other),
    }

    // Verify that a PENDING session was created (even for retry response)
    // Based on service.rs:88-94, retry responses still create a record
    let hashed_phone_retry = test_phone_hasher()
        .hash_phone_number(phone.as_str())
        .unwrap();
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
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;
    let repository = SmsVerificationRepository::new(db.clone(), test_phone_hasher());

    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Scenario 1: mark_verified on already VERIFIED record
    let phone1 = PhoneNumber::new("+30666666666").unwrap();

    setup_prelude_create_verification(&phone1, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(&phone1, "123456", "success")
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
            CreateVerificationRequest {
                phone_number: phone1.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    service
        .send_code(SendCodeRequest {
            phone_number: phone1.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("send_code should succeed");

    // Get the prelude_id and original signup_code
    let record1 = repository
        .get_by_phone_number(&phone1)
        .await
        .expect("Should find verification");
    let prelude_id1 = record1.prelude_id;
    let original_signup_code = record1.signup_code.expect("signup_code should be set");

    // Try to call mark_verified again with different signup code
    let different_signup_code = "different-signup-code";
    let result = repository
        .mark_verified(&prelude_id1, different_signup_code)
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
    let result = repository.mark_failed(&prelude_id1, "test_reason").await;

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
    setup_prelude_create_verification(&phone2, Some(ip), "success", None)
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone2.clone(),
            },
            ip,
        )
        .await
        .expect("create_verification should succeed");

    // Mock expired_or_not_found to reach FAILED state
    setup_prelude_check_code(&phone2, "123456", "expired_or_not_found")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .send_code(SendCodeRequest {
            phone_number: phone2.clone(),
            code: "123456".to_string(),
        })
        .await
        .expect("send_code should succeed");

    // Verify we have a FAILED record before trying to mark it as verified
    let hashed_phone2_mut = test_phone_hasher()
        .hash_phone_number(phone2.as_str())
        .unwrap();
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
    let result = repository.mark_verified(&prelude_id2, signup_code2).await;

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
