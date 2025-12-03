#[cfg(test)]
mod tests {
    use crate::external_apis::homeserver::mock_homeserver_admin_api::MockHomeserverAdminApi;
    use crate::external_apis::prelude::mock_prelude_api::MockSmsVerificationProviderApi;
    use crate::persistence::db::Db;
    use crate::sms_verification::sms_verification_service::{
        SendCodeStatus, SmsVerificationService, VerifyCodeStatus,
    };
    use sqlx::PgPool;

    fn create_mock_service(
        db: Db,
    ) -> SmsVerificationService<MockSmsVerificationProviderApi, MockHomeserverAdminApi> {
        let mock_api = MockSmsVerificationProviderApi::new();
        let mock_signup_token_provider = MockHomeserverAdminApi::new();
        SmsVerificationService::new(mock_api, db, mock_signup_token_provider)
    }

    #[sqlx::test]
    async fn test_full_verification_flow_with_mock(pool: PgPool) {
        // Setup: Mock API + Real Database
        let db = Db::from_pool(pool.clone())
            .await
            .expect("Failed to create Db");
        let service = create_mock_service(db.clone());

        let phone = "+30123456789";

        // Step 1: Initiate verification
        let verify_response = service
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: Some("127.0.0.1".to_string()),
            })
            .await
            .expect("verify_init should succeed");

        assert_eq!(verify_response.status, SendCodeStatus::Success);

        // Step 1.5: Check database after initiation - should have record but not verified yet
        let after_init: (
            String,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<Vec<u8>>,
        ) = sqlx::query_as(
            "SELECT phone_number, prelude_id, verified_at, signup_code FROM sms_verifications WHERE phone_number = $1",
        )
        .bind(phone)
        .fetch_one(db.pool())
        .await
        .expect("Should find verification after init");

        assert_eq!(after_init.0, phone, "Phone number should match");
        let verification_id = after_init.1.clone();
        assert!(
            after_init.2.is_none(),
            "verified_at should be NULL after init"
        );
        assert!(
            after_init.3.is_none(),
            "signup_code should be NULL after init"
        );

        // Step 2: Verify code (mock uses "123456")
        let check_response = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await
            .expect("verify_finalise should succeed");

        assert_eq!(check_response.status, VerifyCodeStatus::Success);

        // Step 3: Query database to verify state updated correctly
        let after_verify: (
            String,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<Vec<u8>>,
        ) = sqlx::query_as(
            "SELECT phone_number, verified_at, signup_code FROM sms_verifications WHERE prelude_id = $1",
        )
        .bind(&verification_id)
        .fetch_one(db.pool())
        .await
        .expect("Should find verification in database");

        assert_eq!(after_verify.0, phone, "Phone number should still match");
        assert!(
            after_verify.1.is_some(),
            "verified_at should be set after successful verification"
        );

        // Check that verified_at is recent (within last minute)
        let verified_at = after_verify.1.unwrap();
        let now = chrono::Utc::now();
        let diff = now - verified_at;
        assert!(
            diff.num_seconds() < 60,
            "verified_at should be recent (was {} seconds ago)",
            diff.num_seconds()
        );

        // Check signup code
        assert!(
            after_verify.2.is_some(),
            "signup_code should be generated after verification"
        );
        let signup_code_bytes = after_verify.2.unwrap();
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
    async fn test_invalid_verification_code(pool: PgPool) {
        let db = Db::from_pool(pool.clone())
            .await
            .expect("Failed to create Db");
        let service = create_mock_service(db.clone());

        let phone = "+30987654321";

        // Initiate verification
        service
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: None,
            })
            .await
            .expect("verify_init should succeed");

        // Try wrong code
        let check_response = service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone.to_string(),
                code: "wrong_code".to_string(),
            })
            .await
            .expect("API should respond");

        assert_eq!(check_response.status, VerifyCodeStatus::Failure);

        // Verify database NOT updated
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1 AND verified_at IS NOT NULL"
        )
        .bind(phone)
        .fetch_one(db.pool())
        .await
        .unwrap();

        assert_eq!(count.0, 0, "Should not mark as verified with wrong code");
    }

    #[sqlx::test]
    async fn test_reuse_active_session(pool: PgPool) {
        let db = Db::from_pool(pool.clone())
            .await
            .expect("Failed to create Db");
        let service = create_mock_service(db.clone());
        let phone = "+30999999999";

        // First call
        let response1 = service
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: None,
            })
            .await
            .expect("First send_code should succeed");

        assert_eq!(response1.status, SendCodeStatus::Success);

        let count1: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count1.0, 1);

        // Second call - should skip DB write since active session exists
        let response2 = service
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: None,
            })
            .await
            .expect("Second send_code should succeed");

        assert_eq!(response2.status, SendCodeStatus::Success);

        let count2: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count2.0, 1, "Should still have only 1 record");
    }

    #[sqlx::test]
    async fn test_new_session_after_verification(pool: PgPool) {
        let db = Db::from_pool(pool.clone())
            .await
            .expect("Failed to create Db");
        let service = create_mock_service(db.clone());
        let phone = "+30000000000";

        service
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: None,
            })
            .await
            .unwrap();

        service
            .verify_code(crate::sms_verification::VerifyCodeRequest {
                phone_number: phone.to_string(),
                code: "123456".to_string(),
            })
            .await
            .unwrap();

        service
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: None,
            })
            .await
            .unwrap();

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sms_verifications WHERE phone_number = $1")
                .bind(phone)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count.0, 2, "Should have 2 records (1 verified, 1 active)");
    }

    #[sqlx::test]
    async fn test_max_verified_sessions_limit(pool: PgPool) {
        let db = Db::from_pool(pool.clone())
            .await
            .expect("Failed to create Db");
        let service = create_mock_service(db.clone());
        let phone = "+30111111112";

        for i in 0..10 {
            service
                .send_code(crate::sms_verification::SendCodeRequest {
                    phone_number: phone.to_string(),
                    ip_address: None,
                })
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
            .send_code(crate::sms_verification::SendCodeRequest {
                phone_number: phone.to_string(),
                ip_address: None,
            })
            .await;

        assert!(result.is_err(), "11th send_code should fail");
        match result {
            Err(crate::sms_verification::error::SmsVerificationError::TooManyVerifiedSessions) => {}
            _ => panic!("Expected TooManyVerifiedSessions error"),
        }
    }
}
