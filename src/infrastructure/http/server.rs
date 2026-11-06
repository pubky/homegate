use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::{
    apple_verification, google_verification,
    infrastructure::{
        config::AppConfig,
        http::HttpServerError,
        sql::{DbError, SqlDb},
    },
    ip_verification, ln_verification,
    shared::{DEPENDENCY_REQUEST_TIMEOUT, HasherArgon2id, HomeserverAdminAPI, Metrics, SignupType},
    sms_verification::http::router,
};

use super::{http_metrics::record_http_metrics, metrics_server::metrics_router};

use axum::{
    Router, error_handling::HandleErrorLayer, http::StatusCode, middleware, response::IntoResponse,
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
// Calls to external services must fail before the request is cut.
const _: () = assert!(DEPENDENCY_REQUEST_TIMEOUT.as_secs() < REQUEST_TIMEOUT.as_secs());

/// Request bodies are small JSON documents (an ID token is a few KiB).
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

pub struct HttpServer {
    pub(crate) http_handle: Handle,
    pub(crate) http_socket: SocketAddr,
    metrics_handle: Option<Handle>,
}

impl HttpServer {
    pub async fn create_router(
        config: &AppConfig,
        db: &SqlDb,
        homeserver_api: &HomeserverAdminAPI,
        hasher: HasherArgon2id,
        metrics: &Metrics,
    ) -> Result<Router, HttpServerError> {
        let mut app = Router::new().route("/", get(root));

        if let Some(sms) = &config.sms_verification {
            tracing::info!("SMS verification enabled");
            app = app.nest(
                "/sms_verification",
                router(
                    homeserver_api,
                    sms,
                    db.clone(),
                    hasher.clone(),
                    metrics.clone(),
                )
                .await?,
            );
        }
        if let Some(ln) = &config.ln_verification {
            tracing::info!("Lightning verification enabled");
            app = app.nest(
                "/ln_verification",
                ln_verification::router(homeserver_api, ln, db.clone(), metrics.clone()).await?,
            );
        }
        if let Some(ip) = &config.ip_verification {
            tracing::info!("IP verification enabled");
            app = app.nest(
                "/ip_verification",
                ip_verification::router(
                    homeserver_api,
                    ip,
                    db.clone(),
                    hasher.clone(),
                    metrics.clone(),
                )
                .await?,
            );
        }
        if let Some(google) = &config.google_verification {
            tracing::info!("Google verification enabled");
            app = app.nest(
                "/google_verification",
                google_verification::router(
                    homeserver_api,
                    google,
                    db.clone(),
                    hasher.clone(),
                    metrics.clone(),
                )
                .await?,
            );
        }
        if let Some(apple) = &config.apple_verification {
            tracing::info!("Apple verification enabled");
            app = app.nest(
                "/apple_verification",
                apple_verification::router(
                    homeserver_api,
                    apple,
                    db.clone(),
                    hasher.clone(),
                    metrics.clone(),
                )
                .await?,
            );
        }

        // Innermost: the timeout and body limit, so the request metrics below
        // also count `request_timeout` (503) and oversized bodies (413).
        app = with_request_timeout(app, REQUEST_TIMEOUT)
            .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES));
        // CORS preflights answered by the CORS layer are not counted, every
        // response of a route is.
        app = app.layer(middleware::from_fn_with_state(
            metrics.clone(),
            record_http_metrics,
        ));

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
                    .allow_headers(Any)
                    .expose_headers([axum::http::header::RETRY_AFTER]),
            )
        } else {
            app
        };

        Ok(app.layer(TraceLayer::new_for_http()))
    }

    pub async fn start(config: AppConfig, hasher: HasherArgon2id) -> Result<Self, HttpServerError> {
        let metrics = Metrics::new();
        let homeserver_api = Self::connect_to_homeserver(&config, &metrics).await;

        let db = SqlDb::connect(&config.database_url)
            .await
            .map_err(DbError::from)?;

        report_enabled_routes(&config, &metrics);
        let router = Self::create_router(&config, &db, &homeserver_api, hasher, &metrics).await?;

        // Bind the metrics listener first so a taken port fails startup before
        // the public listener starts serving.
        let metrics_handle = match &config.metrics {
            Some(metrics_config) => {
                let (handle, socket) =
                    Self::start_http_server(metrics_config.listen_socket, metrics_router(metrics))
                        .await?;
                tracing::info!("Homegate metrics listening on http://{}/metrics", socket);
                Some(handle)
            }
            None => None,
        };

        let (http_handle, http_socket) =
            match Self::start_http_server(config.http_listen_socket, router).await {
                Ok(listener) => listener,
                Err(error) => {
                    // Release the metrics port too, so a retry starts clean.
                    if let Some(handle) = &metrics_handle {
                        handle.shutdown();
                    }
                    return Err(error);
                }
            };

        Ok(Self {
            http_handle,
            http_socket,
            metrics_handle,
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
        if let Some(metrics_handle) = &self.metrics_handle {
            metrics_handle.graceful_shutdown(Some(Duration::from_secs(5)));
        }
    }

    /// Connect to the homeserver admin API, verify credentials, and fetch
    /// the homeserver's public key from /info. Exits the process on failure.
    async fn connect_to_homeserver(config: &AppConfig, metrics: &Metrics) -> HomeserverAdminAPI {
        let mut api = HomeserverAdminAPI::from_config(
            &config.homeserver.admin_api_url,
            &config.homeserver.admin_password,
            metrics.clone(),
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

/// Expose which verification routes this instance has enabled.
fn report_enabled_routes(config: &AppConfig, metrics: &Metrics) {
    metrics.set_route_enabled(SignupType::Sms, config.sms_verification.is_some());
    metrics.set_route_enabled(SignupType::Lightning, config.ln_verification.is_some());
    metrics.set_route_enabled(SignupType::Ip, config.ip_verification.is_some());
    metrics.set_route_enabled(SignupType::Google, config.google_verification.is_some());
    metrics.set_route_enabled(SignupType::Apple, config.apple_verification.is_some());
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
    use axum_test::TestServer;
    use sqlx::PgPool;
    use tower::ServiceExt;

    #[sqlx::test]
    async fn public_router_counts_every_response(pool: PgPool) {
        let config: AppConfig = toml::from_str(
            r#"
database_url = "postgres://localhost:5432/pubky_homegate"

[homeserver]
admin_api_url = "http://localhost:1"
admin_password = "unused"

[ip_verification]
max_verifications_per_week = 1
"#,
        )
        .unwrap();
        let servers = WiremockServers::start().await;
        let metrics = Metrics::new();
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
            metrics.clone(),
        );
        let hasher = HasherArgon2id::new(tempfile::tempdir().unwrap().keep().join("pepper.txt"));
        let router = HttpServer::create_router(
            &config,
            &SqlDb::test(pool).await,
            &homeserver_api,
            hasher,
            &metrics,
        )
        .await
        .unwrap();
        let server =
            TestServer::new(router.into_make_service_with_connect_info::<SocketAddr>()).unwrap();
        setup_homeserver_signup_token("token")
            .expect(1)
            .mount(&servers.homeserver_server)
            .await;

        server.get("/").await.assert_status_ok();
        server.post("/ip_verification").await.assert_status_ok();
        server
            .post("/ip_verification")
            .await
            .assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
        server.get("/nope").await.assert_status_not_found();

        assert_eq!(metrics.http_requests("/", "GET", 200), 1);
        assert_eq!(metrics.http_requests("/ip_verification", "POST", 200), 1);
        assert_eq!(metrics.http_requests("/ip_verification", "POST", 429), 1);
        assert_eq!(metrics.http_requests("unmatched", "GET", 404), 1);
    }

    #[sqlx::test]
    async fn cors_preflights_are_not_counted(pool: PgPool) {
        use axum::http::{Method, StatusCode, header};

        let config: AppConfig = toml::from_str(
            r#"
database_url = "postgres://localhost:5432/pubky_homegate"
allow_cors = true

[homeserver]
admin_api_url = "http://localhost:1"
admin_password = "unused"

[ip_verification]
"#,
        )
        .unwrap();
        let servers = WiremockServers::start().await;
        let metrics = Metrics::new();
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
            metrics.clone(),
        );
        let hasher = HasherArgon2id::new(tempfile::tempdir().unwrap().keep().join("pepper.txt"));
        let router = HttpServer::create_router(
            &config,
            &SqlDb::test(pool).await,
            &homeserver_api,
            hasher,
            &metrics,
        )
        .await
        .unwrap();
        let server =
            TestServer::new(router.into_make_service_with_connect_info::<SocketAddr>()).unwrap();

        server
            .method(Method::OPTIONS, "/ip_verification")
            .add_header(header::ORIGIN, "https://example.com")
            .add_header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .await
            .assert_status(StatusCode::OK);

        // Rendered first: the test reader below creates the series it reads.
        assert!(!metrics.render().unwrap().contains("method=\"OPTIONS\""));
        assert_eq!(metrics.http_requests("/ip_verification", "OPTIONS", 200), 0);
    }

    #[test]
    fn enabled_routes_are_reported_in_metrics() {
        let config: AppConfig = toml::from_str(
            r#"
database_url = "postgres://localhost:5432/pubky_homegate"

[homeserver]
admin_api_url = "http://localhost:1"
admin_password = "unused"

[ip_verification]

[google_verification]
google_client_id = "test-client-id"
"#,
        )
        .unwrap();
        let metrics = Metrics::new();

        report_enabled_routes(&config, &metrics);

        assert_eq!(metrics.route_enabled(SignupType::Ip), 1);
        assert_eq!(metrics.route_enabled(SignupType::Google), 1);
        assert_eq!(metrics.route_enabled(SignupType::Sms), 0);
        assert_eq!(metrics.route_enabled(SignupType::Lightning), 0);
        assert_eq!(metrics.route_enabled(SignupType::Apple), 0);
    }

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
        let metrics = Metrics::new();
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
            metrics.clone(),
        );
        let db = SqlDb::test(pool).await;
        let hasher = HasherArgon2id::with_max_concurrent_hashes(
            tempfile::tempdir().unwrap().keep().join("pepper.txt"),
            max_concurrent_hashes,
        );
        let router =
            HttpServer::create_router(&config, &db, &homeserver_api, hasher.clone(), &metrics)
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

        // Real hashes this time. Which requests get the slot depends on
        // timing, so only check that every one finishes with 200 or 503.
        // (Deterministic 503s are covered by the test above.)
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

    #[sqlx::test]
    async fn test_oversized_streamed_json_body_is_rejected(pool: PgPool) {
        let servers = WiremockServers::start().await;
        let config: AppConfig = toml::from_str(&format!(
            r#"
database_url = "postgres://localhost:5432/unused"

[homeserver]
admin_api_url = "{}"
admin_password = "test-pass"

[google_verification]
google_client_id = "test-client-id"
"#,
            servers.homeserver_server.uri()
        ))
        .unwrap();
        let metrics = Metrics::new();
        let homeserver_api = HomeserverAdminAPI::new(
            &servers.homeserver_server.uri().parse().unwrap(),
            "test-pass",
            "test-homeserver-pubky",
            metrics.clone(),
        );
        let db = SqlDb::test(pool).await;
        let hasher = HasherArgon2id::new(tempfile::tempdir().unwrap().keep().join("pepper.txt"));
        let app = HttpServer::create_router(&config, &db, &homeserver_api, hasher, &metrics)
            .await
            .unwrap();

        // No Content-Length: the limit has to catch the body while it is read.
        let token = "a".repeat(MAX_REQUEST_BODY_BYTES);
        let json = format!(r#"{{"googleIdToken":"{token}"}}"#);
        let chunks: Vec<Result<Vec<u8>, std::io::Error>> = json
            .into_bytes()
            .chunks(4096)
            .map(|c| Ok(c.to_vec()))
            .collect();
        let request = Request::post("/google_verification")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from_stream(futures_util::stream::iter(chunks)))
            .unwrap();
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
