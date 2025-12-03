mod error;
mod routes;

use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use crate::{
    app_state::AppState,
    external_apis::{HomeserverAdminApiTrait, SmsVerificationProviderApi},
};
use axum::{Router, routing::get};
use axum_server::Handle;
use futures_util::TryFutureExt;
use tower_http::trace::TraceLayer;

pub struct HttpServer {
    pub(crate) http_handle: Handle,
    pub(crate) http_socket: SocketAddr,
}

impl HttpServer {
    pub async fn start<T, S>(
        listen_socket: std::net::SocketAddr,
        state: AppState<T, S>,
    ) -> anyhow::Result<Self>
    where
        T: SmsVerificationProviderApi + Clone + 'static,
        S: HomeserverAdminApiTrait + Clone + 'static,
    {
        let router = Self::create_router(state);
        let (http_handle, http_socket) = Self::start_http_server(listen_socket, router).await?;
        Ok(Self {
            http_handle,
            http_socket,
        })
    }

    pub(crate) fn create_router<T, S>(state: AppState<T, S>) -> Router
    where
        T: SmsVerificationProviderApi + Clone + 'static,
        S: HomeserverAdminApiTrait + Clone + 'static,
    {
        base().layer(TraceLayer::new_for_http()).with_state(state)
    }

    /// Start the HTTP server
    async fn start_http_server(
        listen_socket: std::net::SocketAddr,
        router: Router,
    ) -> anyhow::Result<(Handle, SocketAddr)> {
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

fn base<T, S>() -> Router<AppState<T, S>>
where
    T: SmsVerificationProviderApi + Clone + 'static,
    S: HomeserverAdminApiTrait + Clone + 'static,
{
    Router::new()
        .route("/", get(routes::root::handler))
        .nest("/v1", routes::sms_verification::routes())
}
