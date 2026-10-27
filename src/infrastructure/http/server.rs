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
    shared::{HasherArgon2id, HomeserverAdminAPI, Metrics, SignupType},
    sms_verification::http::router,
};

use super::metrics_server::metrics_router;

use axum::{Router, response::IntoResponse, routing::get};
use axum_server::Handle;
use futures_util::TryFutureExt;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

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
            Self::start_http_server(config.http_listen_socket, router).await?;

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
