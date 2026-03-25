use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::{
    EnvConfig,
    infrastructure::{
        http::HttpServerError,
        sql::{DbError, SqlDb},
    },
    ip_verification, ln_verification,
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
    pub fn create_router(
        sms_verification_router: Router,
        ln_verification_router: Router,
        ip_verification_router: Option<Router>,
        allow_cors: bool,
    ) -> Router {
        let mut router = Router::new()
            .route("/", get(root))
            .nest("/ln_verification", ln_verification_router)
            .nest("/sms_verification", sms_verification_router);

        if let Some(ip_router) = ip_verification_router {
            router = router.nest("/ip_verification", ip_router);
        }

        let router = if allow_cors {
            tracing::info!("Enabling CORS for any origin, method, and headers");
            router.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
        } else {
            router
        };

        router.layer(TraceLayer::new_for_http())
    }

    pub async fn start(config: EnvConfig) -> Result<Self, HttpServerError> {
        Self::err_on_homeserver_admin_api_failure(&config).await;

        let db = SqlDb::connect(&config.database_url)
            .await
            .map_err(DbError::from)?;

        let sms_verification_router = router(&config, &db).await?;
        let ln_verification_router = ln_verification::router(&config, &db).await?;
        let ip_verification_router = if config.ip_verification_enabled {
            Some(ip_verification::router(&config, &db).await?)
        } else {
            None
        };
        let router = Self::create_router(
            sms_verification_router,
            ln_verification_router,
            ip_verification_router,
            config.allow_cors,
        );

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

    // Verify homeserver is available at startup and credentials are correct, exit process if not.
    async fn err_on_homeserver_admin_api_failure(config: &EnvConfig) {
        let homeserver_admin_api = HomeserverAdminAPI::new(
            &config.homeserver_admin_api_url,
            &config.homeserver_admin_password,
            &config.homeserver_pubky,
        );
        if let Err(e) = homeserver_admin_api.verify_password().await {
            tracing::error!(
                "Homeserver connection failed: {:?}. Stopping server because credentials are incorrect or homeserver is unavailable.",
                e
            );
            std::process::exit(1);
        }
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
