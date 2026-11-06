use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::{
    google_verification,
    infrastructure::{
        config::AppConfig,
        http::HttpServerError,
        sql::{DbError, SqlDb},
    },
    ip_verification, ln_verification,
    shared::HasherArgon2id,
    shared::HomeserverAdminAPI,
    sms_verification::http::router,
};

use axum::{
    Router, error_handling::HandleErrorLayer, http::StatusCode, response::IntoResponse,
    routing::get,
};
use axum_server::Handle;
use futures_util::TryFutureExt;
use tower::{BoxError, ServiceBuilder};
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};

/// Upper bound for one request; the slowest legitimate route is the 25 s
/// Lightning long poll. Slower requests get 503 `request_timeout`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Request bodies are small JSON documents (an ID token is a few KiB).
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

pub struct HttpServer {
    pub(crate) http_handle: Handle,
    pub(crate) http_socket: SocketAddr,
}

impl HttpServer {
    pub async fn create_router(
        config: &AppConfig,
        db: &SqlDb,
        homeserver_api: &HomeserverAdminAPI,
        hasher: HasherArgon2id,
    ) -> Result<Router, HttpServerError> {
        let mut app = Router::new().route("/", get(root));

        if let Some(sms) = &config.sms_verification {
            tracing::info!("SMS verification enabled");
            app = app.nest(
                "/sms_verification",
                router(homeserver_api, sms, db.clone(), hasher.clone()).await?,
            );
        }
        if let Some(ln) = &config.ln_verification {
            tracing::info!("Lightning verification enabled");
            app = app.nest(
                "/ln_verification",
                ln_verification::router(homeserver_api, ln, db.clone()).await?,
            );
        }
        if let Some(ip) = &config.ip_verification {
            tracing::info!("IP verification enabled");
            app = app.nest(
                "/ip_verification",
                ip_verification::router(homeserver_api, ip, db.clone(), hasher.clone()).await?,
            );
        }
        if let Some(google) = &config.google_verification {
            tracing::info!("Google verification enabled");
            app = app.nest(
                "/google_verification",
                google_verification::router(homeserver_api, google, db.clone(), hasher.clone())
                    .await?,
            );
        }

        let mut app = with_request_timeout(app, REQUEST_TIMEOUT)
            .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES));

        if config.accept_proxy_ip_headers {
            tracing::info!("Accepting proxy IP headers (X-Forwarded-For, X-Real-IP)");
            app = app.layer(axum::Extension(super::AcceptProxyIpHeaders));
        }

        let app = if config.allow_cors {
            tracing::info!("Enabling CORS for any origin, method, and headers");
            app.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
        } else {
            app
        };

        Ok(app.layer(TraceLayer::new_for_http()))
    }

    pub async fn start(config: AppConfig, hasher: HasherArgon2id) -> Result<Self, HttpServerError> {
        let homeserver_api = Self::connect_to_homeserver(&config).await;

        let db = SqlDb::connect(&config.database_url)
            .await
            .map_err(DbError::from)?;

        let router = Self::create_router(&config, &db, &homeserver_api, hasher).await?;

        let (http_handle, http_socket) =
            Self::start_http_server(config.http_listen_socket, router).await?;
        Ok(Self {
            http_handle,
            http_socket,
        })
    }

    /// Start the HTTP server
    async fn start_http_server(
        listen_socket: std::net::SocketAddr,
        router: Router,
    ) -> Result<(Handle, SocketAddr), HttpServerError> {
        let http_listener = TcpListener::bind(listen_socket)?;
        let http_socket = http_listener.local_addr()?;
        let http_handle = Handle::new();
        tokio::spawn(
            axum_server::from_tcp(http_listener)
                .handle(http_handle.clone())
                .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                .map_err(|error| {
                    tracing::error!(?error, "Http server init error");
                }),
        );

        Ok((http_handle, http_socket))
    }

    /// Get the URL of the http server.
    pub fn url_string(&self) -> String {
        format!("http://{}", self.http_socket)
    }

    pub fn shutdown(&self) {
        self.http_handle
            .graceful_shutdown(Some(Duration::from_secs(5)));
    }

    /// Connect to the homeserver admin API, verify credentials, and fetch
    /// the homeserver's public key from /info. Exits the process on failure.
    async fn connect_to_homeserver(config: &AppConfig) -> HomeserverAdminAPI {
        let mut api = HomeserverAdminAPI::from_config(
            &config.homeserver.admin_api_url,
            &config.homeserver.admin_password,
        );
        if let Err(e) = api.fetch_info().await {
            tracing::error!(
                "Homeserver connection failed: {:?}. Stopping server because credentials are incorrect or homeserver is unavailable.",
                e
            );
            std::process::exit(1);
        }
        api
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub async fn root() -> Result<impl IntoResponse, String> {
    Ok("Homegate Service")
}

/// Answers 503 `request_timeout` for any request in `router` that takes
/// longer than `timeout`.
fn with_request_timeout(router: Router, timeout: Duration) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(timeout_response))
            .timeout(timeout),
    )
}

async fn timeout_response(error: BoxError) -> (StatusCode, &'static str) {
    if error.is::<tower::timeout::error::Elapsed>() {
        tracing::warn!("Request timed out");
        (StatusCode::SERVICE_UNAVAILABLE, "request_timeout")
    } else {
        tracing::error!(%error, "Unhandled middleware error");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::{WiremockServers, setup_homeserver_signup_token};
    use axum::{
        body::{Body, to_bytes},
        extract::ConnectInfo,
        http::Request,
        response::Response,
    };
    use sqlx::PgPool;
    use tower::ServiceExt;

    async fn send(router: &Router, request: Request<Body>) -> (StatusCode, String) {
        let response: Response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn get_request(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    fn post_from(uri: &str, client: [u8; 4], body: Body) -> Request<Body> {
        let mut request = Request::post(uri).body(body).unwrap();
        let addr = SocketAddr::from((client, 40000));
        request.extensions_mut().insert(ConnectInfo(addr));
        request
    }

    /// Full router with `/ip_verification` enabled and a hasher capped at
    /// `max_concurrent_hashes`.
    async fn ip_verification_app(
        pool: PgPool,
        servers: &WiremockServers,
        max_concurrent_hashes: usize,
    ) -> (Router, HasherArgon2id) {
        let config: AppConfig = toml::from_str(&format!(
            r#"
database_url = "postgres://localhost:5432/unused"

[homeserver]
admin_api_url = "{}"
admin_password = "test-pass"

[ip_verification]
"#,
            servers.homeserver_server.uri()
        ))
        .unwrap();
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
        );
        let db = SqlDb::test(pool).await;
        let hasher = HasherArgon2id::new(tempfile::tempdir().unwrap().keep().join("pepper.txt"))
            .with_max_concurrent_hashes(max_concurrent_hashes);
        let router = HttpServer::create_router(&config, &db, &homeserver_api, hasher.clone())
            .await
            .unwrap();
        (router, hasher)
    }

    #[sqlx::test]
    async fn test_busy_hasher_returns_503_fast_and_root_stays_up(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (app, hasher) = ip_verification_app(pool, &servers, 2).await;
        setup_homeserver_signup_token("token-after-busy")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        let occupied = hasher.occupy_all_slots();
        let requests = (1..=8u8).map(|i| {
            send(
                &app,
                post_from("/ip_verification", [198, 51, 100, i], Body::empty()),
            )
        });
        let responses = tokio::time::timeout(
            Duration::from_secs(5),
            futures_util::future::join_all(requests),
        )
        .await
        .expect("requests over the hashing cap must not wait for a slot");
        for (status, body) in responses {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(body, "Server busy, please retry");
        }

        let (status, body) = send(&app, get_request("/")).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::OK, "Homegate Service")
        );

        drop(occupied);
        let (status, body) = send(
            &app,
            post_from("/ip_verification", [198, 51, 100, 1], Body::empty()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("token-after-busy"), "{body}");
    }

    #[sqlx::test]
    async fn test_parallel_requests_beyond_hashing_cap_never_hang(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (app, _hasher) = ip_verification_app(pool, &servers, 1).await;
        setup_homeserver_signup_token("token")
            .mount(&servers.homeserver_server)
            .await;

        // Real hashes this time: one runs, the rest are turned away.
        let requests = (1..=6u8).map(|i| {
            send(
                &app,
                post_from("/ip_verification", [198, 51, 100, i], Body::empty()),
            )
        });
        let (responses, root) = tokio::time::timeout(
            Duration::from_secs(30),
            futures_util::future::join(
                futures_util::future::join_all(requests),
                send(&app, get_request("/")),
            ),
        )
        .await
        .expect("no request may hang");

        assert_eq!(root.0, StatusCode::OK);
        let busy = responses
            .iter()
            .filter(|(status, _)| *status == StatusCode::SERVICE_UNAVAILABLE)
            .count();
        let ok = responses
            .iter()
            .filter(|(status, _)| *status == StatusCode::OK)
            .count();
        assert_eq!(busy + ok, responses.len(), "{responses:?}");
        assert!(
            busy >= 1,
            "6 parallel hashes with a cap of 1: {responses:?}"
        );
        assert!(ok >= 1, "{responses:?}");
    }

    #[sqlx::test]
    async fn test_oversized_body_is_rejected(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let (app, _hasher) = ip_verification_app(pool, &servers, 2).await;

        let body = Body::from(vec![b'a'; MAX_REQUEST_BODY_BYTES + 1]);
        let mut request = post_from("/ip_verification", [198, 51, 100, 1], body);
        request.headers_mut().insert(
            axum::http::header::CONTENT_LENGTH,
            (MAX_REQUEST_BODY_BYTES + 1).into(),
        );
        let (status, _) = send(&app, request).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn test_slow_request_times_out_with_503() {
        let slow = || async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            "done"
        };
        let app = with_request_timeout(
            Router::new().route("/slow", get(slow)),
            Duration::from_millis(50),
        );

        assert_eq!(
            send(&app, get_request("/slow")).await,
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "request_timeout".to_string()
            )
        );
    }
}
