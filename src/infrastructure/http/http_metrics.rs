//! Counts and times every response of the public router in [`Metrics`].
//!
//! The `route` label is axum's matched route template (for example
//! `/sms_verification/send_code`), never the raw path, so scanners cannot
//! create unbounded series. Requests that match no route are labelled
//! `unmatched`. The `method` label is limited to the standard methods, with
//! everything else as `other`, because HTTP accepts arbitrary method tokens.

use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request, State},
    http::Method,
    middleware::Next,
    response::Response,
};

use crate::shared::Metrics;

const UNMATCHED_ROUTE: &str = "unmatched";
const OTHER_METHOD: &str = "other";

pub async fn record_http_metrics(
    State(metrics): State<Metrics>,
    matched_path: Option<MatchedPath>,
    request: Request,
    next: Next,
) -> Response {
    let route = matched_path
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| UNMATCHED_ROUTE.to_owned());
    let method = method_label(request.method());
    let start = Instant::now();

    let response = next.run(request).await;

    metrics.record_http_request(&route, method, response.status().as_u16(), start.elapsed());
    response
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => OTHER_METHOD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, middleware, routing::get};
    use axum_test::TestServer;

    #[tokio::test]
    async fn responses_are_counted_by_route_template_and_status() {
        let metrics = Metrics::new();
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .nest(
                "/nested",
                Router::new().route("/{id}", get(|| async { StatusCode::NO_CONTENT })),
            )
            .layer(middleware::from_fn_with_state(
                metrics.clone(),
                record_http_metrics,
            ));
        let server = TestServer::new(app).unwrap();

        server.get("/ok").await.assert_status_ok();
        server.get("/ok").await.assert_status_ok();
        server
            .get("/nested/one")
            .await
            .assert_status(StatusCode::NO_CONTENT);
        server
            .get("/nested/two")
            .await
            .assert_status(StatusCode::NO_CONTENT);
        server.get("/missing").await.assert_status_not_found();

        assert_eq!(metrics.http_requests("/ok", "GET", 200), 2);
        assert_eq!(metrics.http_requests("/nested/{id}", "GET", 204), 2);
        assert_eq!(metrics.http_requests(UNMATCHED_ROUTE, "GET", 404), 1);
    }

    #[tokio::test]
    async fn non_standard_methods_share_one_label() {
        let metrics = Metrics::new();
        let app = Router::new().route("/ok", get(|| async { "ok" })).layer(
            middleware::from_fn_with_state(metrics.clone(), record_http_metrics),
        );
        let server = TestServer::new(app).unwrap();

        for verb in ["BREW", "PROPFIND", "AAAA1"] {
            server
                .method(Method::from_bytes(verb.as_bytes()).unwrap(), "/ok")
                .await
                .assert_status(StatusCode::METHOD_NOT_ALLOWED);
        }

        assert_eq!(metrics.http_requests("/ok", OTHER_METHOD, 405), 3);
        assert!(!metrics.render().unwrap().contains("BREW"));
    }
}
