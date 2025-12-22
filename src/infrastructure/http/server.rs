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
    ln_verification,
    shared::HomeserverObserver,
    sms_verification::http::router,
};

use axum::{Router, response::IntoResponse, routing::get};
use axum_server::Handle;
use futures_util::TryFutureExt;
use tower_http::trace::TraceLayer;

pub struct HttpServer {
    pub(crate) http_handle: Handle,
    pub(crate) http_socket: SocketAddr,
}

impl HttpServer {
    pub fn create_router(
        sms_verification_router: Router,
        ln_verification_router: Router,
    ) -> Router {
        Router::new()
            .route("/", get(root))
            .nest("/sms_verification", sms_verification_router)
            .nest("/ln_verification", ln_verification_router)
            .layer(TraceLayer::new_for_http())
    }

    pub async fn start(config: EnvConfig) -> Result<Self, HttpServerError> {
        HomeserverObserver::spawn_crash_on_unresponsive_homeserver(&config).await;

        let db = SqlDb::connect(&config.database_url)
            .await
            .map_err(DbError::from)?;

        let sms_verification_router = router(&config, &db).await?;
        let ln_verification_router = ln_verification::router(&config, &db).await?;
        let router = Self::create_router(sms_verification_router, ln_verification_router);

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
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub async fn root() -> Result<impl IntoResponse, String> {
    Ok("Homegate Service")
}
