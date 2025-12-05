mod http;

use crate::EnvConfig;
use axum_test::TestServer;
use sqlx::PgPool;

/// Create test server with axum-test and isolated database
pub async fn create_test_server(pool: PgPool) -> (TestServer, PgPool) {
    use std::net::SocketAddr;

    let config = create_test_config();
    let sms_verification_router =
        crate::sms_verification::http::test_router(&config, Some(pool.clone()))
            .await
            .expect("Failed to create test router");

    let router = crate::HttpServer::create_router(sms_verification_router);

    let app = router.into_make_service_with_connect_info::<SocketAddr>();
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, pool)
}

fn create_test_config() -> EnvConfig {
    EnvConfig {
        database_url: Default::default(),
        http_listen_socket: "127.0.0.1:0".parse().unwrap(),
        prelude_api_key: "test-key".to_string(),
        prelude_api_url: "http://localhost".parse().unwrap(),
        homeserver_admin_api_url: "http://localhost".parse().unwrap(),
        homeserver_admin_password: "test-pass".to_string(),
        homeserver_pubky: "test-homeserver-pubky".to_string(),
        max_verified_sessions: 10,
    }
}
