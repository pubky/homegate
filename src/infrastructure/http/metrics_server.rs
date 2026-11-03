//! Serves the shared [`Metrics`] recorder as a Prometheus `/metrics` endpoint.
//!
//! Runs on its own listener (see `[metrics]` in `config.toml`) so it can be
//! kept off the public network and reachable by the monitoring stack only.

use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};

use crate::shared::Metrics;

const PROMETHEUS_TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

pub fn metrics_router(metrics: Metrics) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(metrics)
}

async fn metrics_handler(State(metrics): State<Metrics>) -> impl IntoResponse {
    match metrics.render() {
        Ok(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, PROMETHEUS_TEXT_CONTENT_TYPE)],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "Failed to render metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, PROMETHEUS_TEXT_CONTENT_TYPE)],
                "# Failed to render metrics\n",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::SignupType;
    use axum_test::TestServer;

    #[tokio::test]
    async fn metrics_endpoint_renders_recorder() {
        let metrics = Metrics::new();
        metrics.record_signup_code_issued(SignupType::Ip);
        let server = TestServer::new(metrics_router(metrics)).unwrap();

        let response = server.get("/metrics").await;

        response.assert_status_ok();
        assert_eq!(
            response.header(header::CONTENT_TYPE),
            PROMETHEUS_TEXT_CONTENT_TYPE
        );
        let body = response.text();
        assert!(body.contains("# TYPE homegate_signup_codes_issued_total counter\n"));
        assert!(body.contains("homegate_signup_codes_issued_total{type=\"ip\"} 1\n"));
        assert!(body.contains("homegate_signup_codes_issued_total{type=\"sms\"} 0\n"));
    }
}
