//! Fail-closed panic handling for the enforcement HTTP surface.
//!
//! The service binaries build with the `unwind` panic strategy — the workspace
//! release profile deliberately does NOT set `panic = "abort"` (Plan 05,
//! ADR-1). That lets a [`tower_http::catch_panic::CatchPanicLayer`] turn a
//! reachable handler panic into a fail-closed HTTP 500 for that one request
//! instead of an abort that kills the process — and with it every co-located
//! workload that trusts this enforcement sidecar.

use axum::body::Body;
use axum::http::{Response, StatusCode};

use crate::observability::ERRORS_TOTAL;

/// Deny-equivalent body for a caught panic. Carries no internal detail — the
/// panic message goes to the error log, not the client.
const PANIC_BODY: &str =
    r#"{"error":"internal_error","message":"request handler panicked; the request was denied"}"#;

/// Map a caught handler panic to a fail-closed 500 (never a 2xx), loudly:
/// an error log plus an `errors_total{type="handler_panic"}` increment so
/// alerting keeps the "something is badly wrong" signal without the
/// availability outage an abort would cause.
///
/// Wired as `CatchPanicLayer::custom(catch_panic_response)` on the agent
/// router (Plan 05, Step 1).
pub fn catch_panic_response(err: Box<dyn std::any::Any + Send + 'static>) -> Response<Body> {
    let detail = panic_detail(err.as_ref());
    tracing::error!(panic = %detail, "handler panicked; returning 500 (fail closed)");
    ERRORS_TOTAL.with_label_values(&["handler_panic"]).inc();
    fail_closed_500()
}

/// Best-effort human-readable message from a panic payload, if it carried one.
fn panic_detail(err: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "non-string panic payload".to_string()
    }
}

/// A 500 response with a JSON deny body. The builder can only fail on an
/// invalid status/header, both of which are constants here, so the error arm
/// is unreachable — but if it ever fired we still return a 500, never a 2xx.
fn fail_closed_500() -> Response<Body> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header("content-type", "application/json")
        .body(Body::from(PANIC_BODY))
        .unwrap_or_else(|_| {
            let mut resp = Response::new(Body::from("internal error"));
            *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            resp
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt; // for `oneshot`

    // Explicit return types keep these off the never-type-fallback lint that a
    // bare `async { panic!() }` closure trips.
    async fn boom() -> &'static str {
        panic!("deliberate test panic: boom")
    }

    async fn boom_string_payload() -> &'static str {
        // Format args make the payload a `String` (vs the `&str` from `boom`),
        // so both downcast arms are exercised over the network path.
        let which = "formatted";
        panic!("deliberate {which} panic")
    }

    async fn health() -> &'static str {
        "ok"
    }

    fn panic_router() -> Router {
        Router::new()
            .route("/boom", get(boom))
            .route("/boom-str", get(boom_string_payload))
            .route("/health", get(health))
            .layer(tower_http::catch_panic::CatchPanicLayer::custom(
                catch_panic_response,
            ))
    }

    #[tokio::test]
    async fn panic_becomes_500_and_process_survives() {
        let app = panic_router();

        // A panicking handler returns a 500, not an abort.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/boom")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("service responds instead of aborting");
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = resp
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        // Fail closed: the body is the deny payload, never a success.
        assert!(
            body.starts_with(br#"{"error":"internal_error""#),
            "unexpected body: {}",
            String::from_utf8_lossy(&body)
        );

        // The process is still alive: a follow-up request on the SAME router
        // (same process) succeeds.
        let health = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("process still serving after a caught panic");
        assert_eq!(health.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn string_payload_panic_also_becomes_500() {
        let app = panic_router();
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/boom-str")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("service responds instead of aborting");
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn detail_extracts_string_and_str_payloads() {
        assert_eq!(panic_detail(&String::from("owned")), "owned");
        assert_eq!(panic_detail(&"borrowed"), "borrowed");
        assert_eq!(panic_detail(&42u32), "non-string panic payload");
    }
}
