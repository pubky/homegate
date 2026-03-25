//! Service-layer integration tests for IP verification.
//!
//! These tests verify business logic by calling service methods directly with real database
//! and mocked external APIs (Homeserver). For HTTP-specific concerns (status codes, headers,
//! JSON parsing), add tests to `http.rs` instead.

use crate::e2e::{WiremockServers, setup_homeserver_signup_token};
use crate::infrastructure::sql::SqlDb;
use crate::ip_verification::error::IpVerificationError;
use crate::ip_verification::service::IpVerificationService;
use crate::shared::HomeserverAdminAPI;
use sqlx::PgPool;
use std::net::IpAddr;

fn create_service(
    servers: &WiremockServers,
    max_per_week: u32,
    max_per_year: u32,
) -> IpVerificationService {
    use crate::EnvConfig;

    let config = EnvConfig::for_test(
        servers.prelude_server.uri().parse().unwrap(),
        servers.homeserver_server.uri().parse().unwrap(),
    );

    let homeserver_admin_api = HomeserverAdminAPI::new(
        &config.homeserver_admin_api_url,
        &config.homeserver_admin_password,
        &config.homeserver_pubky,
    );
    IpVerificationService::new(homeserver_admin_api, max_per_week, max_per_year)
}

#[sqlx::test]
async fn test_weekly_window_ages_out(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service(&servers, 2, 10);
    let db = SqlDb::test(pool.clone()).await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    // Allow 3 verifications total (2 now + 1 after aging)
    setup_homeserver_signup_token("token")
        .expect(3)
        .mount(&servers.homeserver_server)
        .await;

    // Use up the weekly limit
    service.verify(&db, ip).await.expect("1st should succeed");
    service.verify(&db, ip).await.expect("2nd should succeed");

    // 3rd should fail — weekly limit hit
    let err = service.verify(&db, ip).await.unwrap_err();
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
    .execute(db.pool())
    .await
    .expect("Failed to age verification");

    // Now only 1 record is within the weekly window — should succeed again
    service
        .verify(&db, ip)
        .await
        .expect("3rd should succeed after aging");
}

#[sqlx::test]
async fn test_rate_limits_are_per_ip(pool: PgPool) {
    let servers = WiremockServers::start().await;
    let service = create_service(&servers, 1, 10);
    let db = SqlDb::test(pool.clone()).await;
    let ip_a: IpAddr = "10.0.0.1".parse().unwrap();
    let ip_b: IpAddr = "10.0.0.2".parse().unwrap();

    // IP A gets 1, IP B gets 1
    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service
        .verify(&db, ip_a)
        .await
        .expect("IP A 1st should succeed");

    // IP A is now at the weekly limit
    let err = service.verify(&db, ip_a).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::WeeklyLimitExceeded),
        "IP A should be rate-limited, got: {err:?}"
    );

    // IP B should still work — independent limit
    service
        .verify(&db, ip_b)
        .await
        .expect("IP B 1st should succeed");
}

#[sqlx::test]
async fn test_annual_limit_persists_after_weekly_window(pool: PgPool) {
    let servers = WiremockServers::start().await;
    // weekly=10 (high), annual=2 — so only the annual limit matters
    let service = create_service(&servers, 10, 2);
    let db = SqlDb::test(pool.clone()).await;
    let ip: IpAddr = "10.0.0.1".parse().unwrap();

    setup_homeserver_signup_token("token")
        .expect(2)
        .mount(&servers.homeserver_server)
        .await;

    service.verify(&db, ip).await.expect("1st should succeed");
    service.verify(&db, ip).await.expect("2nd should succeed");

    // Age both records outside the weekly window but inside the annual window
    let eight_days_ago = chrono::Utc::now().naive_utc() - chrono::Duration::days(8);
    sqlx::query("UPDATE ip_verifications SET created_at = $1")
        .bind(eight_days_ago)
        .execute(db.pool())
        .await
        .expect("Failed to age verifications");

    // Even though weekly window is clear, annual limit should block
    let err = service.verify(&db, ip).await.unwrap_err();
    assert!(
        matches!(err, IpVerificationError::AnnualLimitExceeded),
        "Expected AnnualLimitExceeded, got: {err:?}"
    );
}
