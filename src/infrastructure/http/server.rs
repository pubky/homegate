use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::{
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
