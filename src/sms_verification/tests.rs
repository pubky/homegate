#[cfg(test)]
mod tests {
    use crate::infrastructure::database::SqlDb;
    use crate::shared::homeserver::mock_homeserver_admin_api::MockHomeserverAdminApi;
    use crate::sms_verification::prelude_api::{
        MockSmsVerificationProviderApi, PreludeSendCodeStatus, PreludeVerifyCodeStatus,
    };
    use crate::sms_verification::repository::{
        SmsVerificationRepository, SmsVerificationRepositoryError,
    };
    use crate::sms_verification::service::SmsVerificationService;
    use sqlx::PgPool;
    use std::net::IpAddr;

    fn create_mock_service(
        db: SqlDb,
    ) -> SmsVerificationService<MockSmsVerificationProviderApi, MockHomeserverAdminApi> {
        let mock_api = MockSmsVerificationProviderApi::new();
        let mock_signup_token_provider = MockHomeserverAdminApi::new();
        let repository = SmsVerificationRepository::new(db);
        SmsVerificationService::new(repository, mock_api, mock_signup_token_provider, 10)
    }

    #[sqlx::test]
    async fn test_full_verification_flow_with_mock(pool: PgPool) {
        // Setup: Mock API + Real Database
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());

        let phone = "+30123456789";

        // Step 1: Initiate verification
        let verify_response = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("verify_init should succeed");

        assert_eq!(verify_response.status, PreludeSendCodeStatus::Success);

        // Step 1.5: Check database after initiation - should have record but not verified yet
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

        // Step 2: Verify code (mock uses "123456")
        let check_response = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await
            .expect("verify_finalise should succeed");

        assert_eq!(check_response.status, PreludeVerifyCodeStatus::Success);

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
            signup_code_str.len() > 0,
            "signup_code string should not be empty"
        );
    }

    #[sqlx::test]
    async fn test_session_lifecycle(pool: PgPool) {
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());

        // Test 1: Active session reuse
        let phone1 = "+30999999999";

        // First send_code creates active session
        let response1 = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone1.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("First send_code should succeed");

        assert_eq!(response1.status, PreludeSendCodeStatus::Success);

        let count1: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone1)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count1.0, 1, "Should have 1 active session");

        // Second send_code reuses active session (no new DB record)
        let response2 = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone1.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("Second send_code should succeed");

        assert_eq!(response2.status, PreludeSendCodeStatus::Retry);

        let count2: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone1)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count2.0, 1, "Should still have only 1 record (reused)");

        // Test 2: New session after verification
        let phone2 = "+30000000000";

        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone2.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("send_code should succeed");

        service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone2.to_string(),
                code: "123456".to_string(),
            })
            .await
            .expect("verify_code should succeed");

        // After verification, new send_code creates a new session
        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone2.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
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
    async fn test_max_verified_sessions_limit(pool: PgPool) {
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());
        let phone = "+30111111112";

        for i in 0..10 {
            service
                .send_code(
                    crate::sms_verification::SendCodeRequest {
                        phone_number: phone.to_string(),
                    },
                    "127.0.0.1".parse::<IpAddr>().unwrap(),
                )
                .await
                .expect(&format!("send_code {} should succeed", i));

            service
                .verify_code(crate::sms_verification::VerifyCodeRequest {
                    phone_number: phone.to_string(),
                    code: "123456".to_string(),
                })
                .await
                .expect(&format!("verify_code {} should succeed", i));
        }

        let result = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await;

        assert!(result.is_err(), "11th send_code should fail");
        match result {
            Err(crate::sms_verification::error::SmsVerificationError::TooManyVerifiedSessions) => {}
            _ => panic!("Expected TooManyVerifiedSessions error"),
        }
    }

    #[sqlx::test]
    async fn test_input_validation_and_errors(pool: PgPool) {
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());

        // Invalid phone - send_code
        let result = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: "invalid-phone".to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await;
        assert!(
            matches!(
                result,
                Err(crate::sms_verification::error::SmsVerificationError::InvalidPhoneNumber(_))
            ),
            "send_code should reject invalid phone format"
        );

        // Invalid phone - verify_code
        let result = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: "+0123456789".to_string(), // starts with 0
                code: "123456".to_string(),
            })
            .await;
        assert!(
            matches!(
                result,
                Err(crate::sms_verification::error::SmsVerificationError::InvalidPhoneNumber(_))
            ),
            "verify_code should reject invalid phone format"
        );

        // Invalid verification code - should return Failure status
        let phone_wrong_code = "+30987654321";
        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone_wrong_code.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("send_code should succeed");

        let check_response = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone_wrong_code.to_string(),
                code: "wrong_code".to_string(),
            })
            .await
            .expect("API should respond");

        assert_eq!(
            check_response.status,
            PreludeVerifyCodeStatus::Failure,
            "Wrong code should return Failure status"
        );

        // Verify database NOT updated when wrong code provided
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1 AND status = 'VERIFIED'"
        )
        .bind(phone_wrong_code)
        .fetch_one(db.pool())
        .await
        .unwrap();

        assert_eq!(count.0, 0, "Should not mark as verified with wrong code");

        // Boundary: 9 verified sessions should allow 10th
        let phone = "+30888888888";
        for i in 0..9 {
            service
                .send_code(
                    crate::sms_verification::SendCodeRequest {
                        phone_number: phone.to_string(),
                    },
                    "127.0.0.1".parse::<IpAddr>().unwrap(),
                )
                .await
                .expect(&format!("send_code {} should succeed", i));
            service
                .verify_code(crate::sms_verification::VerifyCodeRequest {
                    phone_number: phone.to_string(),
                    code: "123456".to_string(),
                })
                .await
                .expect(&format!("verify_code {} should succeed", i));
        }
        let result = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await;
        assert!(result.is_ok(), "10th verification should succeed");

        // At limit: 10 verified sessions should reject 11th
        service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await
            .expect("verify_code for 10th session should succeed");
        let result = service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await;
        assert!(
            matches!(
                result,
                Err(crate::sms_verification::error::SmsVerificationError::TooManyVerifiedSessions)
            ),
            "11th send_code should fail with TooManyVerifiedSessions"
        );
    }

    #[sqlx::test]
    async fn test_expired_or_not_found_marks_failed(pool: PgPool) {
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());

        let phone = "+30666666666";

        // Step 1: Create a verification session
        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("send_code should succeed");

        // Step 2: Manually delete from mock's state to simulate expired session
        // This will cause check_code to not find the verification, but we need
        // to test the actual flow where Prelude returns "expired_or_not_found"
        // For now, let's verify the database state after we manually mark it as failed

        // Get the prelude_id from DB
        let (prelude_id, status_before): (String, String) = sqlx::query_as(
            "SELECT prelude_id, status FROM sms_verifications WHERE phone_number = $1",
        )
        .bind(phone)
        .fetch_one(db.pool())
        .await
        .expect("Should find verification record");

        assert_eq!(status_before, "PENDING", "Should start as PENDING");

        // Step 3: Manually mark as failed (simulating what would happen if Prelude returned expired_or_not_found)
        let repository = SmsVerificationRepository::new(db.clone());
        repository
            .mark_failed(&prelude_id, "expired_or_not_found")
            .await
            .expect("Should mark as failed");

        // Step 4: Verify database state
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

        // Step 5: Verify that check_pending_verification_exists returns NoActiveVerification
        let result = repository.check_pending_exists(phone).await;
        assert!(
            matches!(
                result,
                Err(SmsVerificationRepositoryError::NoActiveVerification(_))
            ),
            "Failed sessions should not be considered active"
        );

        // Step 6: Verify that we can create a new session after failure
        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
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
    async fn test_verify_code_with_wrong_phone_number(pool: PgPool) {
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());

        let phone_send = "+30666666666";
        let phone_verify = "+30888888889";

        // Step 1: Send verification code to phone_send
        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone_send.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("send_code should succeed");

        // Step 2: Try to verify with a different phone number
        let result = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone_verify.to_string(),
                code: "123456".to_string(),
            })
            .await;

        // Should fail with NoActiveVerification error since there's no pending verification for phone_verify
        match result {
            Err(crate::sms_verification::error::SmsVerificationError::DatabaseError(
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
    async fn test_database_error_handling(pool: PgPool) {
        let db = SqlDb::test(pool.clone()).await;
        let service = create_mock_service(db.clone());

        let phone = "+30777777777";

        // First, create a session with send_code so the mock API knows about this phone
        service
            .send_code(
                crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                },
                "127.0.0.1".parse::<IpAddr>().unwrap(),
            )
            .await
            .expect("send_code should succeed");

        // Now manually delete the database record to simulate database inconsistency
        sqlx::query("DELETE FROM sms_verifications WHERE phone_number = $1")
            .bind(phone)
            .execute(db.pool())
            .await
            .expect("Failed to delete record");

        // Now try to verify - mock will succeed but database lookup will fail
        let result = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await;

        // Should propagate NoActiveVerification error from database through service layer
        match result {
            Err(crate::sms_verification::error::SmsVerificationError::DatabaseError(
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
}
