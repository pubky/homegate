//! Service-layer integration tests for Google verification.
//!
//! These tests use a fake Google verifier and mocked Homeserver API. They do not
//! call Google or depend on live JWKS endpoints.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::e2e::{
    WiremockServers, setup_homeserver_signup_token, setup_homeserver_signup_token_with_quota,
};
use crate::infrastructure::config::{GoogleVerificationConfig, SignupQuotaConfig};
use crate::infrastructure::sql::SqlDb;
use crate::shared::HomeserverAdminAPI;

use super::error::GoogleVerificationError;
use super::google_id_token_verifier::{
    GoogleIdTokenVerificationError, GoogleIdTokenVerifier, VerifiedGoogleIdentity,
};
use super::service::GoogleVerificationService;

static TEST_HASHER: std::sync::LazyLock<crate::shared::HasherArgon2id> =
    std::sync::LazyLock::new(|| {
        let dir = tempfile::tempdir().unwrap();
        crate::shared::HasherArgon2id::new(dir.path().join("pepper.txt"))
    });

fn create_service(
    servers: &WiremockServers,
    db: SqlDb,
    max_per_week: u32,
    max_per_year: u32,
) -> GoogleVerificationService {
    create_service_with_verifier(
        servers,
        db,
        max_per_week,
        max_per_year,
        None,
        fake_valid_verifier("google-subject"),
    )
}

fn create_service_with_quota(
    servers: &WiremockServers,
    db: SqlDb,
    signup_quota: SignupQuotaConfig,
) -> GoogleVerificationService {
    create_service_with_verifier(
        servers,
        db,
        2,
        4,
        Some(signup_quota),
        fake_valid_verifier("google-subject"),
    )
}

fn create_service_with_verifier(
    servers: &WiremockServers,
    db: SqlDb,
    max_per_week: u32,
    max_per_year: u32,
    signup_quota: Option<SignupQuotaConfig>,
    verifier: Arc<dyn GoogleIdTokenVerifier>,
) -> GoogleVerificationService {
    let homeserver_admin_api = HomeserverAdminAPI::new(
        &servers.homeserver_server.uri().parse().unwrap(),
        "test-pass",
        "test-homeserver-pubky",
    );
    let config = GoogleVerificationConfig {
        google_client_id: "test-google-client-id.apps.googleusercontent.com".to_string(),
        max_verifications_per_week: max_per_week,
        max_verifications_per_year: max_per_year,
        signup_quota,
    };
    GoogleVerificationService::with_verifier(
        db,
        homeserver_admin_api,
        &config,
        TEST_HASHER.clone(),
        verifier,
    )
}

fn fake_valid_verifier(subject: &str) -> Arc<dyn GoogleIdTokenVerifier> {
    Arc::new(FakeGoogleVerifier {
        result: Ok(VerifiedGoogleIdentity {
            issuer: "https://accounts.google.com".to_string(),
            subject: subject.to_string(),
        }),
    })
}

fn fake_error_verifier(error: GoogleIdTokenVerificationError) -> Arc<dyn GoogleIdTokenVerifier> {
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
async fn test_successful_verification(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 2, 4);

    setup_homeserver_signup_token("token-123")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    let response = service
        .verify("valid-google-id-token")
        .await
        .expect("verification should succeed");

    assert_eq!(response.signup_code, "token-123");
    assert_eq!(response.homeserver_pubky, "test-homeserver-pubky");

    let stored_hash: String =
        sqlx::query_scalar("SELECT google_identity_hash FROM google_verifications")
            .fetch_one(&pool)
            .await
            .expect("verification row should exist");
    assert_ne!(stored_hash, "google-subject");
    assert!(!stored_hash.contains("google-subject"));
}

#[sqlx::test]
async fn test_weekly_limit_exceeded(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, db, 1, 10);

    setup_homeserver_signup_token("token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service
        .verify("valid-google-id-token")
        .await
        .expect("first verification should succeed");

    let error = service.verify("valid-google-id-token").await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::WeeklyLimitExceeded
    ));
}

#[sqlx::test]
async fn test_rate_limits_are_per_google_identity(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service_a = create_service_with_verifier(
        &servers,
        db.clone(),
        1,
        10,
        None,
        fake_valid_verifier("google-subject-a"),
    );
    let service_b = create_service_with_verifier(
        &servers,
        db,
        1,
        10,
        None,
        fake_valid_verifier("google-subject-b"),
    );

    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service_a
        .verify("valid-google-id-token-a")
        .await
        .expect("first identity should pass");
    service_b
        .verify("valid-google-id-token-b")
        .await
        .expect("second identity should pass independently");

    let error = service_a
        .verify("valid-google-id-token-a")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::WeeklyLimitExceeded
    ));
}

#[sqlx::test]
async fn test_annual_limit_exceeded(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 10, 1);

    setup_homeserver_signup_token("token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service
        .verify("valid-google-id-token")
        .await
        .expect("first verification should succeed");

    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query("UPDATE google_verifications SET created_at = $1")
        .bind(eight_days_ago)
        .execute(&pool)
        .await
        .expect("Failed to age verification");

    let error = service.verify("valid-google-id-token").await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::AnnualLimitExceeded
    ));
}

#[sqlx::test]
async fn test_signup_quota_uses_post_endpoint(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service_with_quota(
        &servers,
        db,
        SignupQuotaConfig {
            storage_quota_mb: Some(64),
            rate_read: Some("1mb/s".to_string()),
            rate_read_burst: Some(10),
            rate_write: Some("1mb/s".to_string()),
            rate_write_burst: Some(10),
            allowed_write_paths: Some(vec!["/pub/".to_string()]),
        },
    );

    setup_homeserver_signup_token_with_quota("quota-token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    let response = service
        .verify("valid-google-id-token")
        .await
        .expect("verification should succeed");
    assert_eq!(response.signup_code, "quota-token");
}

#[sqlx::test]
async fn test_invalid_google_token_does_not_call_homeserver(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service_with_verifier(
        &servers,
        db,
        2,
        4,
        None,
        fake_error_verifier(GoogleIdTokenVerificationError::Invalid),
    );

    let error = service.verify("invalid-google-id-token").await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::InvalidGoogleIdToken
    ));
}

#[sqlx::test]
async fn test_google_verifier_unavailable_does_not_call_homeserver(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service_with_verifier(
        &servers,
        db,
        2,
        4,
        None,
        fake_error_verifier(GoogleIdTokenVerificationError::DependencyUnavailable),
    );

    let error = service.verify("valid-google-id-token").await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::GoogleVerifierUnavailable
    ));
}

#[sqlx::test]
async fn test_homeserver_unavailable_returns_error(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, db, 2, 4);

    let error = service.verify("valid-google-id-token").await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::HomeserverUnavailable
    ));
}

#[sqlx::test]
async fn test_concurrent_requests_respect_rate_limit(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, db, 1, 10);

    setup_homeserver_signup_token("token")
        .expect(1..=2)
        .mount(&servers.homeserver_server)
        .await;

    let service_a = service.clone();
    let service_b = service.clone();
    let (result_a, result_b) = tokio::join!(
        service_a.verify("valid-google-id-token"),
        service_b.verify("valid-google-id-token"),
    );

    let results = [result_a, result_b];
    let success_count = results.iter().filter(|result| result.is_ok()).count();
    let weekly_limit_count = results
        .iter()
        .filter(|result| matches!(result, Err(GoogleVerificationError::WeeklyLimitExceeded)))
        .count();

    assert_eq!(success_count, 1);
    assert_eq!(weekly_limit_count, 1);
}
