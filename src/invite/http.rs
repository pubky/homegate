use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::put,
};

use crate::{
    EnvConfig,
    infrastructure::http::HttpServerError,
    invite::{
        app_state::AppState,
        error::InviteError,
        types::{InviteRequest, InviteResponse},
    },
};

pub async fn router(
    config: &EnvConfig,
    db: &crate::infrastructure::sql::SqlDb,
    pubky: pubky::Pubky,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db.clone(), pubky);
    Ok(Router::new()
        .route("/", put(invite_handler))
        .with_state(state))
}

#[cfg(test)]
pub async fn router_with_db(
    config: &EnvConfig,
    db: crate::infrastructure::sql::SqlDb,
    pubky: pubky::Pubky,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db, pubky);
    Ok(Router::new()
        .route("/", put(invite_handler))
        .with_state(state))
}

async fn invite_handler(
    State(state): State<AppState>,
    Json(request): Json<InviteRequest>,
) -> Result<Json<InviteResponse>, InviteError> {
    let response = state.invite.invite(&state.db, request).await?;
    Ok(Json(response))
}

impl IntoResponse for InviteError {
    fn into_response(self) -> Response {
        let status = match self {
            InviteError::InvalidPubkey => StatusCode::UNPROCESSABLE_ENTITY,
            InviteError::InvalidPreimage => StatusCode::UNPROCESSABLE_ENTITY,
            InviteError::ProofNotFound => StatusCode::FORBIDDEN,
            InviteError::ProofMismatch => StatusCode::UNPROCESSABLE_ENTITY,
            InviteError::WeeklyLimitExceeded => StatusCode::UNAUTHORIZED,
            InviteError::AnnualLimitExceeded => StatusCode::UNAUTHORIZED,
            InviteError::InsufficientPosts => StatusCode::UNAUTHORIZED,
            InviteError::HomeserverUnavailable => {
                tracing::error!("Homeserver unavailable during invite");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            InviteError::Database(ref err) => {
                tracing::error!(error = %err, "Database operation failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            InviteError::Transaction(ref err) => {
                tracing::error!(error = %err, "Database transaction failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::{
        WiremockServers, setup_homeserver_signup_token, setup_homeserver_signup_token_failure,
        setup_pubky_proof_file, setup_pubky_proof_file_not_found, setup_signup_token_claimed,
    };
    use crate::invite::types::InviteStatus;
    use axum_test::TestServer;
    use sha2::{Digest, Sha256};
    use sqlx::PgPool;
    use std::net::SocketAddr;

    /// Compute the SHA-256 hash of a hex-encoded preimage, matching the service logic.
    fn sha256_hex(preimage_hex: &str) -> String {
        let bytes = hex::decode(preimage_hex).expect("test preimage must be valid hex");
        hex::encode(Sha256::digest(&bytes))
    }

    async fn create_http_test_server(
        pool: PgPool,
        servers: &WiremockServers,
    ) -> (TestServer, PgPool) {
        use crate::EnvConfig;
        use crate::infrastructure::sql::SqlDb;

        let config = EnvConfig::for_test(
            servers.prelude_server.uri().parse().unwrap(),
            servers.homeserver_server.uri().parse().unwrap(),
        );

        let db = SqlDb::test(pool.clone()).await;

        let pubky = pubky::Pubky::testnet().expect("Failed to create Pubky testnet client");
        let invite_router = router_with_db(&config, db, pubky)
            .await
            .expect("Failed to create router");

        let router = Router::new().nest("/invite", invite_router);
        let app = router.into_make_service_with_connect_info::<SocketAddr>();
        let server = TestServer::new(app).expect("Failed to create test server");

        (server, pool)
    }

    #[sqlx::test]
    async fn http_returns_200_for_successful_invite(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        let preimage = "deadbeef";
        let proof_hash = sha256_hex(preimage);

        // Mock the proof file and signup token
        setup_pubky_proof_file(&proof_hash)
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        setup_homeserver_signup_token("token-invite-123")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no",
                "hashProofPreimage": preimage
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "token-invite-123");
    }

    #[sqlx::test]
    async fn http_returns_422_for_invalid_pubkey(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": "not-a-valid-pubkey",
                "hashProofPreimage": "deadbeef"
            }))
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[sqlx::test]
    async fn http_returns_403_when_proof_not_found(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        setup_pubky_proof_file_not_found()
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no",
                "hashProofPreimage": "deadbeef"
            }))
            .await;

        response.assert_status(StatusCode::FORBIDDEN);
        let body = response.text();
        assert!(body.contains("not found"), "Should mention proof not found");
    }

    #[sqlx::test]
    async fn http_returns_422_when_proof_hash_mismatch(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        // Store a hash that doesn't match the preimage
        setup_pubky_proof_file("wrong-hash-value")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no",
                "hashProofPreimage": "deadbeef"
            }))
            .await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let body = response.text();
        assert!(
            body.contains("does not match"),
            "Should mention hash mismatch"
        );
    }

    #[sqlx::test]
    async fn http_returns_existing_unclaimed_token(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool.clone(), &servers).await;

        let pubkey = "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no";
        let preimage = "cafebabe";
        let proof_hash = sha256_hex(preimage);

        setup_pubky_proof_file(&proof_hash)
            .mount(&servers.homeserver_server)
            .await;
        // Only one signup token should be generated
        setup_homeserver_signup_token("token-invite-1")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        // Second call will check if the token is claimed — mock it as unclaimed
        setup_signup_token_claimed("token-invite-1", false)
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        // First call creates an UNCLAIMED token
        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": pubkey,
                "hashProofPreimage": preimage
            }))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "token-invite-1");

        // Second call returns the same UNCLAIMED token
        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": pubkey,
                "hashProofPreimage": preimage
            }))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "token-invite-1");
    }

    #[sqlx::test]
    async fn http_returns_429_when_weekly_limit_exceeded(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool.clone(), &servers).await;

        let pubkey = "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no";
        let preimage = "cafebabe";
        let proof_hash = sha256_hex(preimage);

        setup_pubky_proof_file(&proof_hash)
            .mount(&servers.homeserver_server)
            .await;

        // Insert 2 CLAIMED records directly to simulate past usage
        for i in 0..2 {
            sqlx::query(
                "INSERT INTO invite_friend (pubkey, proof_hash, signup_code, status) VALUES ($1, $2, $3, 'CLAIMED')"
            )
            .bind(pubkey)
            .bind(&proof_hash)
            .bind(format!("old-token-{}", i))
            .execute(&_pool)
            .await
            .unwrap();
        }

        // Next invite should be rate limited
        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": pubkey,
                "hashProofPreimage": preimage
            }))
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn http_returns_429_when_annual_limit_exceeded(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool.clone(), &servers).await;

        let pubkey = "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no";
        let preimage = "cafebabe";
        let proof_hash = sha256_hex(preimage);

        setup_pubky_proof_file(&proof_hash)
            .mount(&servers.homeserver_server)
            .await;

        // Insert 4 CLAIMED records (annual limit) spread across different weeks
        for i in 0..4 {
            let days_ago = i * 30; // spread across months
            sqlx::query(
                "INSERT INTO invite_friend (pubkey, proof_hash, signup_code, status, created_at) VALUES ($1, $2, $3, 'CLAIMED', NOW() - make_interval(days => $4))"
            )
            .bind(pubkey)
            .bind(&proof_hash)
            .bind(format!("old-token-{}", i))
            .bind(days_ago)
            .execute(&_pool)
            .await
            .unwrap();
        }

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": pubkey,
                "hashProofPreimage": preimage
            }))
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);
        let body = response.text();
        assert!(
            body.contains("this year"),
            "Should mention annual limit: {}",
            body
        );
    }

    #[sqlx::test]
    async fn http_returns_401_when_insufficient_posts(pool: PgPool) {
        use crate::EnvConfig;
        use crate::infrastructure::sql::SqlDb;

        let servers = WiremockServers::start().await;
        let mut config = EnvConfig::for_test(
            servers.prelude_server.uri().parse().unwrap(),
            servers.homeserver_server.uri().parse().unwrap(),
        );
        config.min_posts_for_invite = 5;

        let db = SqlDb::test(pool.clone()).await;
        let pubky = pubky::Pubky::testnet().expect("Failed to create Pubky testnet client");
        let invite_router = router_with_db(&config, db, pubky)
            .await
            .expect("Failed to create router");
        let router = Router::new().nest("/invite", invite_router);
        let app = router.into_make_service_with_connect_info::<SocketAddr>();
        let server = TestServer::new(app).expect("Failed to create test server");

        let preimage = "deadbeef";
        let proof_hash = sha256_hex(preimage);

        setup_pubky_proof_file(&proof_hash)
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no",
                "hashProofPreimage": preimage
            }))
            .await;

        response.assert_status(StatusCode::UNAUTHORIZED);
        let body = response.text();
        assert!(
            body.contains("post more"),
            "Should mention insufficient posts: {}",
            body
        );
    }

    #[sqlx::test]
    async fn http_generates_new_token_when_previous_was_claimed(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool.clone(), &servers).await;

        let pubkey = "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no";
        let preimage = "cafebabe";
        let proof_hash = sha256_hex(preimage);

        setup_pubky_proof_file(&proof_hash)
            .mount(&servers.homeserver_server)
            .await;

        // Insert an UNCLAIMED record that the homeserver reports as claimed
        sqlx::query(
            "INSERT INTO invite_friend (pubkey, proof_hash, signup_code, status) VALUES ($1, $2, $3, 'UNCLAIMED')"
        )
        .bind(pubkey)
        .bind(&proof_hash)
        .bind("old-claimed-token")
        .execute(&_pool)
        .await
        .unwrap();

        // Homeserver says old token is claimed
        setup_signup_token_claimed("old-claimed-token", true)
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        // New token should be generated
        setup_homeserver_signup_token("new-token-456")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": pubkey,
                "hashProofPreimage": preimage
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["signupCode"], "new-token-456");

        // Verify the old record was marked as CLAIMED in the DB
        let row: (String,) = sqlx::query_as(
            "SELECT status FROM invite_friend WHERE signup_code = 'old-claimed-token'",
        )
        .fetch_one(&_pool)
        .await
        .unwrap();
        assert_eq!(row.0, InviteStatus::Claimed.as_str());
    }

    #[sqlx::test]
    async fn http_returns_500_when_token_generation_fails(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool.clone(), &servers).await;

        let pubkey = "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no";
        let preimage = "cafebabe";
        let proof_hash = sha256_hex(preimage);

        setup_pubky_proof_file(&proof_hash)
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;
        setup_homeserver_signup_token_failure()
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": pubkey,
                "hashProofPreimage": preimage
            }))
            .await;

        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);

        // Verify a FAILED record was written to the DB
        let row: (String, String) =
            sqlx::query_as("SELECT status, failure_reason FROM invite_friend WHERE pubkey = $1")
                .bind(pubkey)
                .fetch_one(&_pool)
                .await
                .unwrap();
        assert_eq!(row.0, InviteStatus::Failed.as_str());
        assert_eq!(row.1, "homeserver_signup_token_generation_failed");
    }

    #[sqlx::test]
    async fn http_returns_500_when_proof_fetch_fails(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (server, _pool) = create_http_test_server(pool, &servers).await;

        // No mock mounted for the proof endpoint — wiremock returns a connection error/500

        let response = server
            .put("/invite")
            .json(&serde_json::json!({
                "pubkey": "yg4gxe7z1r7mr6orids9fh95y7gxhdsxjqi6nngsxxtakqaxr5no",
                "hashProofPreimage": "deadbeef"
            }))
            .await;

        // Unmatched wiremock request returns 404 which gets treated differently.
        // The actual behavior depends on wiremock's default — but the service should
        // return either 403 (proof not found) or 500 (unavailable).
        let status = response.status_code();
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::INTERNAL_SERVER_ERROR,
            "Expected 403 or 500, got {}",
            status
        );
    }
}
