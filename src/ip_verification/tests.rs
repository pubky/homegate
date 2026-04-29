//! Service-layer integration tests for IP verification.
//!
//! These tests verify business logic by calling service methods directly with real database
//! and mocked external APIs (Homeserver). For HTTP-specific concerns (status codes, headers,
//! JSON parsing), add tests to `http.rs` instead.

use crate::e2e::{
    WiremockServers, setup_homeserver_signup_token, setup_homeserver_signup_token_with_quota,
};
use crate::infrastructure::config::{IpVerificationConfig, SignupQuotaConfig};
use crate::infrastructure::sql::SqlDb;
use crate::ip_verification::error::IpVerificationError;
use crate::ip_verification::service::IpVerificationService;
use crate::shared::HomeserverAdminAPI;
use sqlx::PgPool;
use std::net::IpAddr;

static TEST_PEPPER_DIR: std::sync::LazyLock<tempfile::TempDir> =
    std::sync::LazyLock::new(|| tempfile::tempdir().unwrap());

fn test_pepper_path() -> std::path::PathBuf {
    TEST_PEPPER_DIR.path().join("pepper.txt")
}

fn create_service(
    servers: &WiremockServers,
    db: SqlDb,
    max_per_week: u32,
    max_per_year: u32,
) -> IpVerificationService {
    create_service_with_quota(servers, db, max_per_week, max_per_year, None)
}

fn create_service_with_quota(
    servers: &WiremockServers,
    db: SqlDb,
    max_per_week: u32,
    max_per_year: u32,
    signup_quota: Option<SignupQuotaConfig>,
) -> IpVerificationService {
    let homeserver_admin_api = HomeserverAdminAPI::new(
        &servers.homeserver_server.uri().parse().unwrap(),
        "test-pass",
        "test-homeserver-pubky",
    );
    let config = IpVerificationConfig {
        max_verifications_per_week: max_per_week,
        max_verifications_per_year: max_per_year,
        signup_quota,
        limit_whitelist: vec![],
    };
    IpVerificationService::new(db, homeserver_admin_api, &config, test_pepper_path())
}

#[sqlx::test]
async fn test_weekly_window_ages_out(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 2, 10);
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    // Allow 3 verifications total (2 now + 1 after aging)
    setup_homeserver_signup_token("token")
        .expect(3)
        .mount(&servers.homeserver_server)
        .await;

    // Use up the weekly limit
    service.verify(ip).await.expect("1st should succeed");
    service.verify(ip).await.expect("2nd should succeed");

    // 3rd should fail — weekly limit hit
    let err = service.verify(ip).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::WeeklyLimitExceeded),
        "Expected WeeklyLimitExceeded, got: {err:?}"
    );

    // Age one record to 8 days ago (outside the 7-day window)
    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query(
        "UPDATE ip_verifications
         SET created_at = $1
         WHERE id = (
             SELECT id FROM ip_verifications
             ORDER BY created_at ASC
             LIMIT 1
         )",
    )
    .bind(eight_days_ago)
    .execute(&pool)
    .await
    .expect("Failed to age verification");

    // Now only 1 record is within the weekly window — should succeed again
    service
        .verify(ip)
        .await
        .expect("3rd should succeed after aging");
}

/// Tests that the annual window ages out after 365 days, allowing new verifications.
#[sqlx::test]
async fn test_annual_window_ages_out(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    // weekly=10 (high), annual=1 — so only the annual limit matters
    let service = create_service(&servers, db, 10, 1);
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    // Allow 2 verifications total (1 now + 1 after aging)
    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service.verify(ip).await.expect("1st should succeed");

    // Should fail — annual limit hit
    let err = service.verify(ip).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::AnnualLimitExceeded),
        "Expected AnnualLimitExceeded, got: {err:?}"
    );

    // Age the record to 366 days ago (outside the 365-day window)
    let past = chrono::Utc::now().naive_utc() - chrono::Duration::days(366);
    sqlx::query("UPDATE ip_verifications SET created_at = $1")
        .bind(past)
        .execute(&pool)
        .await
        .expect("Failed to age verification");

    // Now the annual window is clear — should succeed again
    service
        .verify(ip)
        .await
        .expect("Should succeed after annual window ages out");
}

#[sqlx::test]
async fn test_rate_limits_are_per_ip(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 1, 10);
    let ip_a: IpAddr = "10.0.0.1".parse().unwrap();
    let ip_b: IpAddr = "10.0.0.2".parse().unwrap();

    // IP A gets 1, IP B gets 1
    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service.verify(ip_a).await.expect("IP A 1st should succeed");

    // IP A is now at the weekly limit
    let err = service.verify(ip_a).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::WeeklyLimitExceeded),
        "IP A should be rate-limited, got: {err:?}"
    );

    // IP B should still work — independent limit
    service.verify(ip_b).await.expect("IP B 1st should succeed");
}

#[sqlx::test]
async fn test_annual_limit_persists_after_weekly_window(pool: PgPool) {
    let servers = WiremockServers::start().await;
    // weekly=10 (high), annual=2 — so only the annual limit matters
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 10, 2);
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service.verify(ip).await.expect("1st should succeed");
    service.verify(ip).await.expect("2nd should succeed");

    // Age both records outside the weekly window but inside the annual window
    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query("UPDATE ip_verifications SET created_at = $1")
        .bind(eight_days_ago)
        .execute(&pool)
        .await
        .expect("Failed to age verifications");

    // Even though weekly window is clear, annual limit should block
    let err = service.verify(ip).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::AnnualLimitExceeded),
        "Expected AnnualLimitExceeded, got: {err:?}"
    );
}

/// Verifies the advisory lock prevents concurrent requests for the same IP
/// from both succeeding when only one slot remains. Without the lock, both
/// tasks would read count=0, pass the rate check, and insert — exceeding
/// the limit.
#[sqlx::test]
async fn test_concurrent_requests_respect_rate_limit(pool: PgPool) {
    let servers = WiremockServers::start().await;
    // weekly limit = 1, so only one of the two concurrent requests should succeed
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 1, 10);
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    // The homeserver mock should only be called once (the winning request).
    // Use up to 2 to avoid the mock itself causing a misleading failure —
    // the assertion below is what actually validates correctness.
    setup_homeserver_signup_token("token")
        .expect(1..=2)
        .mount(&servers.homeserver_server)
        .await;

    let service_a = service.clone();
    let service_b = service.clone();

    // Spawn two concurrent verification requests for the same IP
    let handle_a = tokio::spawn(async move { service_a.verify(ip).await });
    let handle_b = tokio::spawn(async move { service_b.verify(ip).await });

    let result_a = handle_a.await.expect("task A panicked");
    let result_b = handle_b.await.expect("task B panicked");

    // Exactly one should succeed and one should fail with WeeklyLimitExceeded
    let (successes, failures): (Vec<_>, Vec<_>) =
        [result_a, result_b].into_iter().partition(Result::is_ok);

    assert_eq!(
        successes.len(),
        1,
        "Exactly one concurrent request should succeed, got {} successes",
        successes.len()
    );
    assert_eq!(
        failures.len(),
        1,
        "Exactly one concurrent request should be rate-limited, got {} failures",
        failures.len()
    );

    let err = failures.into_iter().next().unwrap().unwrap_err();
    assert!(
        matches!(err, IpVerificationError::WeeklyLimitExceeded),
        "Rate-limited request should get WeeklyLimitExceeded, got: {err:?}"
    );
}

#[sqlx::test]
async fn test_ipv6_rate_limiting(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let service = create_service(&servers, db, 1, 10);
    let ipv6: IpAddr = "2001:db8::1".parse().unwrap();

    setup_homeserver_signup_token("token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service.verify(ipv6).await.expect("IPv6 1st should succeed");

    let err = service.verify(ipv6).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::WeeklyLimitExceeded),
        "IPv6 should be rate-limited, got: {err:?}"
    );
}

/// Tests that whitelisted IPs bypass verification rate limits.
#[sqlx::test]
async fn test_whitelist_bypasses_limits(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let mut service = create_service(&servers, db, 1, 2);
    let ip: IpAddr = "10.0.0.50".parse().unwrap();

    service.set_limit_whitelist(vec![ip]);

    // Allow 5 verifications — well beyond both weekly (1) and annual (2) limits
    setup_homeserver_signup_token("token")
        .expect(5)
        .mount(&servers.homeserver_server)
        .await;

    for i in 1..=5 {
        service.verify(ip).await.unwrap_or_else(|e| {
            panic!("Verification {i} should succeed for whitelisted IP, got: {e:?}")
        });
    }
}

/// Tests that non-whitelisted IPs are still rate-limited when a whitelist is configured.
#[sqlx::test]
async fn test_whitelist_does_not_affect_other_ips(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let mut service = create_service(&servers, db, 1, 10);
    let whitelisted_ip: IpAddr = "10.0.0.50".parse().unwrap();
    let other_ip: IpAddr = "10.0.0.51".parse().unwrap();

    service.set_limit_whitelist(vec![whitelisted_ip]);

    setup_homeserver_signup_token("token")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    service.verify(other_ip).await.expect("1st should succeed");

    let err = service.verify(other_ip).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::WeeklyLimitExceeded),
        "Non-whitelisted IP should be rate-limited, got: {err:?}"
    );
}

/// Tests that when signup_quota is configured, the service uses the POST endpoint
/// (generate_signup_token_with_quota) instead of the GET endpoint.
#[sqlx::test]
async fn test_signup_quota_uses_post_endpoint(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let db = SqlDb::test(pool.clone()).await;
    let quota = SignupQuotaConfig {
        storage_quota_mb: Some(64),
        rate_read: Some("1mb/s".to_string()),
        rate_read_burst: None,
        rate_write: Some("1mb/s".to_string()),
        rate_write_burst: None,
    };
    let service = create_service_with_quota(&servers, db, 2, 10, Some(quota));
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    // Only mount the POST mock — if the service incorrectly uses GET, this will fail
    setup_homeserver_signup_token_with_quota("quota-token-123")
        .expect(1)
        .mount(&servers.homeserver_server)
        .await;

    let response = service.verify(ip).await.expect("Should succeed with quota");
    assert_eq!(response.signup_code, "quota-token-123");
}
