//! Service-layer integration tests for SMS verification.
//!
//! These tests verify business logic by calling service methods directly with real database
//! and mocked external APIs (Prelude, Homeserver). Add tests here for business rules, state
//! transitions, and edge cases. For HTTP-specific concerns (status codes, headers, JSON parsing),
//! add tests to `http.rs` instead.

use crate::e2e::{create_service_with_mocked_apis, wiremock_helpers::*};
use crate::infrastructure::database::SqlDb;
use crate::sms_verification::repository::{
    SmsVerificationRepository, SmsVerificationRepositoryError,
};
use crate::sms_verification::{
    CreateVerificationRequest, CreateVerificationResponse, SendCodeRequest, SendCodeResponse,
};
use sqlx::PgPool;
use std::net::IpAddr;

#[sqlx::test]
async fn test_service_full_verification_flow(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    let phone = "+30123456789";
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Setup wiremock expectations
    setup_prelude_create_verification(phone, Some("127.0.0.1"), "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(phone, "123456", "success")
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
                phone_number: phone.to_string(),
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
    let after_init: (
        String,
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<Vec<u8>>,
        String,
    ) = sqlx::query_as(
        "SELECT phone_number, prelude_id, finalised_at, signup_code, status FROM sms_verifications WHERE phone_number = $1",
    )
    .bind(phone)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification after init");

    assert_eq!(after_init.0, phone, "Phone number should match");
    let verification_id = after_init.1.clone();
    assert!(
        after_init.2.is_none(),
        "finalised_at should be NULL after init"
    );
    assert!(
        after_init.3.is_none(),
        "signup_code should be NULL after init"
    );
    assert_eq!(
        after_init.4, "PENDING",
        "status should be PENDING after init"
    );

    // Step 2: Verify code
    let check_response = service
        .send_code(SendCodeRequest {
            phone_number: phone.to_string(),
            code: "123456".to_string(),
        })
        .await
        .expect("verify_finalise should succeed");

    assert!(matches!(check_response, SendCodeResponse::Success { .. }));

    // Step 3: Query database to verify state updated correctly
    let after_verify: (
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<Vec<u8>>,
        String,
    ) = sqlx::query_as(
        "SELECT phone_number, finalised_at, signup_code, status FROM sms_verifications WHERE prelude_id = $1",
    )
    .bind(&verification_id)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification in database");

    assert_eq!(after_verify.0, phone, "Phone number should still match");
    assert!(
        after_verify.1.is_some(),
        "finalised_at should be set after successful verification"
    );

    // Check that finalised_at is recent (within last minute)
    let finalised_at = after_verify.1.unwrap();
    let now = chrono::Utc::now();
    let diff_secs = (now.timestamp() - finalised_at.timestamp()).abs();
    assert!(
        diff_secs < 60,
        "finalised_at should be recent (was {} seconds ago)",
        diff_secs
    );

    // Check signup code
    assert!(
        after_verify.2.is_some(),
        "signup_code should be generated after verification"
    );
    let signup_code_bytes = after_verify.2.unwrap();
    assert_eq!(
        after_verify.3, "VERIFIED",
        "status should be VERIFIED after successful verification"
    );
    assert!(
        !signup_code_bytes.is_empty(),
        "signup_code should not be empty"
    );

    // Verify it's a valid UTF-8 string (since we generate UUID)
    let signup_code_str =
        String::from_utf8(signup_code_bytes).expect("signup_code should be valid UTF-8");
    assert!(
        !signup_code_str.is_empty(),
        "signup_code string should not be empty"
    );
}

#[sqlx::test]
async fn test_service_session_lifecycle(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service_with_mocked_apis(pool.clone(), &servers).await;
    let db = SqlDb::test(pool.clone()).await;

    // Test 1: Active session reuse
    let phone1 = "+30999999999";
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Setup mock - we can use "success" for both calls
    // The important thing is that the DB correctly handles session reuse
    setup_prelude_create_verification(phone1, Some("127.0.0.1"), "success")
        .expect(2) // Both calls will use this mock
        .mount(&servers.prelude_server)
        .await;

    // First send_code creates active session
    let response1 = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone1.to_string(),
            },
            ip,
        )
        .await
        .expect("First send_code should succeed");

    assert!(matches!(response1, CreateVerificationResponse::Success));

    let count1: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
            .bind(phone1)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count1.0, 1, "Should have 1 active session");

    // Second send_code - API might return success again, but our code should
    // see the existing pending session in DB and not create a duplicate
    let _response2 = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone1.to_string(),
            },
            ip,
        )
        .await
        .expect("Second send_code should succeed");

    // Key assertion: DB should still have only 1 record (no duplicate created)
    let count2: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
            .bind(phone1)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count2.0, 1, "Should still have only 1 record (reused)");

    // Test 2: New session after verification
    let phone2 = "+30000000000";

    // Setup mocks for full verification flow + retry after verification
    // We'll call send_code twice (once before verification, once after)
    setup_prelude_create_verification(phone2, Some("127.0.0.1"), "success")
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(phone2, "123456", "success")
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
                phone_number: phone2.to_string(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    service
        .send_code(SendCodeRequest {
            phone_number: phone2.to_string(),
            code: "123456".to_string(),
        })
        .await
        .expect("verify_code should succeed");

    // After verification, new send_code creates a new session
    // Mock already set up above with expect(2)
    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone2.to_string(),
            },
            ip,
        )
        .await
        .expect("send_code after verification should succeed");

    let count3: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
            .bind(phone2)
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
    let phone = "+30111111112";
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Setup mocks for 10 successful verifications
    setup_prelude_create_verification(phone, Some("127.0.0.1"), "success")
        .expect(10)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(phone, "123456", "success")
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
                    phone_number: phone.to_string(),
                },
                ip,
            )
            .await
            .expect(&format!("send_code {} should succeed", i));

        service
            .send_code(SendCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await
            .expect(&format!("verify_code {} should succeed", i));
    }

    // 11th attempt should fail (no mock needed - validation happens before API call)
    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.to_string(),
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

    // Invalid phone - send_code (no mock needed - validation before API)
    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: "invalid-phone".to_string(),
            },
            ip,
        )
        .await;
    assert!(
        matches!(
            result,
            Err(crate::SmsVerificationError::InvalidPhoneNumber(_))
        ),
        "send_code should reject invalid phone format"
    );

    // Invalid phone - verify_code (no mock needed - validation before API)
    let result = service
        .send_code(SendCodeRequest {
            phone_number: "+0123456789".to_string(), // starts with 0
            code: "123456".to_string(),
        })
        .await;
    assert!(
        matches!(
            result,
            Err(crate::SmsVerificationError::InvalidPhoneNumber(_))
        ),
        "verify_code should reject invalid phone format"
    );

    // Invalid verification code - should return Failure status
    let phone_wrong_code = "+30987654321";

    setup_prelude_create_verification(phone_wrong_code, Some("127.0.0.1"), "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(phone_wrong_code, "wrong_code", "failure")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone_wrong_code.to_string(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    let check_response = service
        .send_code(SendCodeRequest {
            phone_number: phone_wrong_code.to_string(),
            code: "wrong_code".to_string(),
        })
        .await
        .expect("API should respond");

    assert!(
        matches!(check_response, SendCodeResponse::Failure),
        "Wrong code should return Failure status"
    );

    // Verify database NOT updated when wrong code provided
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1 AND status = 'VERIFIED'",
    )
    .bind(phone_wrong_code)
    .fetch_one(db.pool())
    .await
    .unwrap();

    assert_eq!(count.0, 0, "Should not mark as verified with wrong code");

    // Boundary: 9 verified sessions should allow 10th
    let phone = "+30888888888";

    // Setup mocks for 10 verifications (9 + 1 more)
    setup_prelude_create_verification(phone, Some("127.0.0.1"), "success")
        .expect(10)
        .mount(&servers.prelude_server)
        .await;

    setup_prelude_check_code(phone, "123456", "success")
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
                    phone_number: phone.to_string(),
                },
                ip,
            )
            .await
            .expect(&format!("send_code {} should succeed", i));
        service
            .send_code(SendCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await
            .expect(&format!("verify_code {} should succeed", i));
    }

    // 10th should succeed

    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.to_string(),
            },
            ip,
        )
        .await;
    assert!(result.is_ok(), "10th verification should succeed");

    // Complete the 10th verification (mocks already set up above with expect(10))
    service
        .send_code(SendCodeRequest {
            phone_number: phone.to_string(),
            code: "123456".to_string(),
        })
        .await
        .expect("verify_code for 10th session should succeed");

    // At limit: 10 verified sessions should reject 11th (no mock needed)
    let result = service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.to_string(),
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

    let phone = "+30666666666";
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Step 1: Create a verification session
    // We'll call send_code twice (once here, once after marking failed), so expect(2)
    setup_prelude_create_verification(phone, Some("127.0.0.1"), "success")
        .expect(2)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.to_string(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    // Step 2: Verify initial status
    let status_before: String =
        sqlx::query_scalar("SELECT status FROM sms_verifications WHERE phone_number = $1")
            .bind(phone)
            .fetch_one(db.pool())
            .await
            .expect("Should find verification record");

    assert_eq!(status_before, "PENDING", "Should start as PENDING");

    // Step 3: Mock Prelude check_code to return expired_or_not_found
    setup_prelude_check_code(phone, "123456", "expired_or_not_found")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    // Step 4: Try to verify with code - this should trigger mark_failed
    let verify_result = service
        .send_code(SendCodeRequest {
            phone_number: phone.to_string(),
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
    let (status, failure_reason, finalised_at): (
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, failure_reason, finalised_at FROM sms_verifications WHERE phone_number = $1",
    )
    .bind(phone)
    .fetch_one(db.pool())
    .await
    .expect("Should find verification record");

    assert_eq!(status, "FAILED", "status should be FAILED");
    assert_eq!(
        failure_reason,
        Some("expired_or_not_found".to_string()),
        "failure_reason should be set"
    );
    assert!(finalised_at.is_some(), "finalised_at should be set");

    // Step 6: Verify that check_pending_verification_exists returns NoActiveVerification
    let repository = SmsVerificationRepository::new(db.clone());
    let result = repository.check_pending_exists(phone).await;
    assert!(
        matches!(
            result,
            Err(SmsVerificationRepositoryError::NoActiveVerification(_))
        ),
        "Failed sessions should not be considered active"
    );

    // Step 7: Verify that we can create a new session after failure
    // Mock already set up above with expect(2)
    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.to_string(),
            },
            ip,
        )
        .await
        .expect("Should be able to create new session after failure");

    // Verify we now have 2 records (1 FAILED, 1 PENDING)
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
            .bind(phone)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count.0, 2, "Should have 2 records after retry");

    let pending_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1 AND status = 'PENDING'",
    )
    .bind(phone)
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

    let phone_send = "+30666666666";
    let phone_verify = "+30888888889";
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // Step 1: Send verification code to phone_send
    setup_prelude_create_verification(phone_send, Some("127.0.0.1"), "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone_send.to_string(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    // Step 2: Try to verify with a different phone number (no mock needed - DB lookup fails first)
    let result = service
        .send_code(SendCodeRequest {
            phone_number: phone_verify.to_string(),
            code: "123456".to_string(),
        })
        .await;

    // Should fail with NoActiveVerification error since there's no pending verification for phone_verify
    match result {
        Err(crate::SmsVerificationError::DatabaseError(
            SmsVerificationRepositoryError::NoActiveVerification(_),
        )) => {
            // Test passes - correct error type
        }
        other => panic!(
            "Expected DatabaseError(NoActiveVerification), got: {:?}",
            other
        ),
    }

    // Step 3: Verify that the original phone number still has a pending verification
    let repository = SmsVerificationRepository::new(db.clone());
    let check_result = repository.check_pending_exists(phone_send).await;
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

    let phone = "+30777777777";
    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // First, create a session with send_code
    setup_prelude_create_verification(phone, Some("127.0.0.1"), "success")
        .expect(1)
        .mount(&servers.prelude_server)
        .await;

    service
        .create_verification(
            CreateVerificationRequest {
                phone_number: phone.to_string(),
            },
            ip,
        )
        .await
        .expect("send_code should succeed");

    // Now manually delete the database record to simulate database inconsistency
    sqlx::query("DELETE FROM sms_verifications WHERE phone_number = $1")
        .bind(phone)
        .execute(db.pool())
        .await
        .expect("Failed to delete record");

    // Setup mock - it will succeed but database lookup will fail
    setup_prelude_check_code(phone, "123456", "success")
        .expect(0) // Should not be called - DB lookup fails first
        .mount(&servers.prelude_server)
        .await;

    // Now try to verify - database lookup fails before API call
    let result = service
        .send_code(SendCodeRequest {
            phone_number: phone.to_string(),
            code: "123456".to_string(),
        })
        .await;

    // Should propagate NoActiveVerification error from database through service layer
    match result {
        Err(crate::SmsVerificationError::DatabaseError(
            SmsVerificationRepositoryError::NoActiveVerification(_),
        )) => {
            // Test passes - database error was properly propagated
        }
        other => panic!(
            "Expected DatabaseError(NoActiveVerification), got: {:?}",
            other
        ),
    }
}
