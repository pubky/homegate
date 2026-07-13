//! Service-layer integration tests for Google verification.
//!
//! These tests use mocked Google JWKS and Homeserver HTTP APIs. They do not call
//! Google or depend on live JWKS endpoints.

use sqlx::PgPool;
use wiremock::MockServer;

use crate::e2e::{WiremockServers, setup_homeserver_signup_token};
use crate::infrastructure::config::GoogleVerificationConfig;
use crate::infrastructure::sql::SqlDb;
use crate::shared::HomeserverAdminAPI;

use super::error::GoogleVerificationError;
use super::google_id_token_verifier::test_support::{
    TEST_GOOGLE_CLIENT_ID, jwks_server, jwks_url, valid_token, wrong_audience_token,
};
use super::service::GoogleVerificationService;

static TEST_HASHER: std::sync::LazyLock<crate::shared::HasherArgon2id> =
    std::sync::LazyLock::new(|| {
        let dir = tempfile::tempdir().unwrap();
        crate::shared::HasherArgon2id::new(dir.path().join("pepper.txt"))
    });

fn create_service(
    servers: &WiremockServers,
    google_server: &MockServer,
    db: SqlDb,
    max_per_week: u32,
    max_per_year: u32,
) -> GoogleVerificationService {
    create_service_with_google_server(servers, google_server, db, max_per_week, max_per_year)
}

fn create_service_with_google_server(
    servers: &WiremockServers,
    google_server: &MockServer,
    db: SqlDb,
    max_per_week: u32,
    max_per_year: u32,
) -> GoogleVerificationService {
    let homeserver_admin_api = HomeserverAdminAPI::new(
        &servers.homeserver_server.uri().parse().unwrap(),
        "test-pass",
        "test-homeserver-pubky",
    );
    let config = GoogleVerificationConfig {
        google_client_id: TEST_GOOGLE_CLIENT_ID.to_string(),
        max_verifications_per_week: max_per_week,
        max_verifications_per_year: max_per_year,
    };
    GoogleVerificationService::for_test(
        db,
        homeserver_admin_api,
        &config,
        TEST_HASHER.clone(),
        jwks_url(google_server),
    )
}

#[sqlx::test]
async fn test_successful_verification(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, &google_server, db, 2, 4);

    setup_homeserver_signup_token("token-123")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    let response = service
        .verify(&valid_token("google-subject"))
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
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, &google_server, db, 1, 10);
    let token = valid_token("google-subject");

    setup_homeserver_signup_token("token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service
        .verify(&token)
        .await
        .expect("first verification should succeed");

    let error = service.verify(&token).await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::WeeklyLimitExceeded
    ));
}

#[sqlx::test]
async fn test_rate_limits_are_per_google_identity(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool).await;
    let service_a = create_service(&servers, &google_server, db.clone(), 1, 10);
    let service_b = create_service(&servers, &google_server, db, 1, 10);
    let token_a = valid_token("google-subject-a");
    let token_b = valid_token("google-subject-b");

    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service_a
        .verify(&token_a)
        .await
        .expect("first identity should pass");
    service_b
        .verify(&token_b)
        .await
        .expect("second identity should pass independently");

    let error = service_a.verify(&token_a).await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::WeeklyLimitExceeded
    ));
}

#[sqlx::test]
async fn test_annual_limit_exceeded(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, &google_server, db, 10, 1);
    let token = valid_token("google-subject");

    setup_homeserver_signup_token("token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service
        .verify(&token)
        .await
        .expect("first verification should succeed");

    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query("UPDATE google_verifications SET created_at = $1")
        .bind(eight_days_ago)
        .execute(&pool)
        .await
        .expect("Failed to age verification");

    let error = service.verify(&token).await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::AnnualLimitExceeded
    ));
}

#[sqlx::test]
async fn test_invalid_google_token_does_not_call_homeserver(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, &google_server, db, 2, 4);

    let error = service.verify(&wrong_audience_token()).await.unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::InvalidGoogleIdToken
    ));
}

#[sqlx::test]
async fn test_google_verifier_unavailable_does_not_call_homeserver(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = MockServer::start().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, &google_server, db, 2, 4);

    let error = service
        .verify(&valid_token("google-subject"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::GoogleVerifierUnavailable
    ));
}

#[sqlx::test]
async fn test_homeserver_unavailable_returns_error(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, &google_server, db, 2, 4);

    let error = service
        .verify(&valid_token("google-subject"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GoogleVerificationError::HomeserverUnavailable
    ));
}

#[sqlx::test]
async fn test_concurrent_requests_respect_rate_limit(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let google_server = jwks_server().await;
    let db = SqlDb::test(pool).await;
    let service = create_service(&servers, &google_server, db, 1, 10);
    let token = valid_token("google-subject");

    setup_homeserver_signup_token("token")
        .expect(1..=2)
        .mount(&servers.homeserver_server)
        .await;

    let service_a = service.clone();
    let service_b = service.clone();
    let token_a = token.clone();
    let (result_a, result_b) = tokio::join!(service_a.verify(&token_a), service_b.verify(&token),);

    let results = [result_a, result_b];
    let success_count = results.iter().filter(|result| result.is_ok()).count();
    let weekly_limit_count = results
        .iter()
        .filter(|result| matches!(result, Err(GoogleVerificationError::WeeklyLimitExceeded)))
        .count();

    assert_eq!(success_count, 1);
    assert_eq!(weekly_limit_count, 1);
}
