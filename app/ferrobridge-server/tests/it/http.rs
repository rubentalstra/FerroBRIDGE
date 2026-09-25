// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HTTP surface and the middleware stack: the routes, the request id, the
//! panic body, the timeout, the body ceiling and the one request log line.

use crate::support;
use axum::Router;
use axum::body::Body;
use axum::routing::get;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
use ferrobridge_server::telemetry::{Rendering, subscriber};
use ferrobridge_server::{request_id, with_middleware};
use http::{Request, Response, StatusCode, header};
use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt as _;

/// Sends `request` through `app` and reads the whole answer.
async fn call(
    app: Router,
    request: Request<Body>,
) -> Result<(StatusCode, String), Box<dyn StdError>> {
    let response: Response<Body> = app.oneshot(request).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
    Ok((status, String::from_utf8(bytes.to_vec())?))
}

/// Sends `request` through `app` and returns the whole response.
async fn raw(app: Router, request: Request<Body>) -> Result<Response<Body>, Box<dyn StdError>> {
    Ok(app.oneshot(request).await?)
}

/// The application under test, with no indicator registered.
fn app() -> Router {
    ferrobridge_server::router(support::state(), &support::settings())
}

#[tokio::test]
async fn the_info_route_serves_the_build_facts_and_the_pins() -> Result<(), Box<dyn StdError>> {
    let (status, body) = call(app(), Request::get("/health/info").body(Body::empty())?).await?;
    assert_eq!(StatusCode::OK, status);
    let document: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(
        Some(env!("CARGO_PKG_VERSION")),
        document["version"].as_str()
    );
    assert_eq!(
        Some(ferrobridge_server::build_info::COMMIT),
        document["build"]["commit"].as_str()
    );
    assert_eq!(
        Some(ferrobridge_server::build_info::BUILT_AT),
        document["build"]["built_at"].as_str()
    );
    assert_eq!(
        Some(ferrobridge_server::build_info::RUSTC),
        document["build"]["rustc"].as_str()
    );
    let pins = document["pins"].as_array().expect("the pins are a list");
    let names: Vec<&str> = pins.iter().filter_map(|pin| pin["name"].as_str()).collect();
    assert_eq!(
        vec![
            "FHIRconnect",
            "FHIR",
            "OMOCL",
            "OMOP CDM",
            "openEHR ITS-REST",
            "openehr-* crates",
            "fhir-types"
        ],
        names
    );
    let keys = |value: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    };
    assert_eq!(
        vec!["build", "pins", "product", "version"],
        keys(&document),
        "the document carries the facts and nothing else"
    );
    assert_eq!(
        vec!["built_at", "commit", "commit_short", "rustc"],
        keys(&document["build"]),
        "the build facts carry no path and no secret"
    );
    Ok(())
}

#[tokio::test]
async fn the_root_document_names_the_product_and_the_version() -> Result<(), Box<dyn StdError>> {
    let (status, body) = call(app(), Request::get("/").body(Body::empty())?).await?;
    assert_eq!(StatusCode::OK, status);
    let document: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("FerroBRIDGE"), document["product"].as_str());
    assert_eq!(
        Some(env!("CARGO_PKG_VERSION")),
        document["version"].as_str()
    );
    assert!(!body.contains('<'), "the root document is JSON, never HTML");
    Ok(())
}

#[tokio::test]
async fn liveness_is_two_hundred_once_the_router_is_built() -> Result<(), Box<dyn StdError>> {
    let (status, _) = call(app(), Request::get("/health/liveness").body(Body::empty())?).await?;
    assert_eq!(StatusCode::OK, status);
    Ok(())
}

#[tokio::test]
async fn readiness_with_no_indicator_is_two_hundred() -> Result<(), Box<dyn StdError>> {
    let (status, body) = call(
        app(),
        Request::get("/health/readiness").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status);
    let document: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("up"), document["state"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_legal_request_id_is_echoed_and_an_illegal_one_is_replaced()
-> Result<(), Box<dyn StdError>> {
    let response = raw(
        app(),
        Request::get("/health/liveness")
            .header(request_id::HEADER, "corr-42")
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        Some("corr-42"),
        response
            .headers()
            .get(request_id::HEADER)
            .and_then(|value| value.to_str().ok())
    );

    // A newline cannot be built as a header value at all, so the illegal
    // forms a wire can carry are an empty value, an over-long one and a
    // non-ASCII byte.
    for illegal in [
        header::HeaderValue::from_static(""),
        header::HeaderValue::from_str(&"a".repeat(request_id::MAX_LENGTH + 1))?,
        header::HeaderValue::from_bytes(&[0xff, 0xfe])?,
    ] {
        let response = raw(
            app(),
            Request::get("/health/liveness")
                .header(request_id::HEADER, illegal.clone())
                .body(Body::empty())?,
        )
        .await?;
        let minted = response
            .headers()
            .get(request_id::HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or("a request id is always answered")?;
        assert!(
            request_id::is_legal(minted),
            "the minted id is legal: {minted}"
        );
        assert_ne!(illegal.as_bytes(), minted.as_bytes());
    }
    Ok(())
}

#[tokio::test]
async fn a_panicking_handler_yields_a_five_hundred_with_the_request_id_in_its_body()
-> Result<(), Box<dyn StdError>> {
    let router = with_middleware(
        Router::new().route(
            "/boom",
            get(|| async {
                panic!("the handler gave up");
                #[expect(unreachable_code, reason = "the route panics by design")]
                StatusCode::OK
            }),
        ),
        support::state(),
        &support::settings(),
    );

    let response = raw(
        router,
        Request::get("/boom")
            .header(request_id::HEADER, "corr-panic")
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
    assert_eq!(
        Some("corr-panic"),
        response
            .headers()
            .get(request_id::HEADER)
            .and_then(|value| value.to_str().ok())
    );
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
    let document: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(Some("internal"), document["error"].as_str());
    assert_eq!(Some("corr-panic"), document["request_id"].as_str());
    assert!(
        !String::from_utf8_lossy(&bytes).contains("the handler gave up"),
        "the panic message never reaches the client"
    );
    Ok(())
}

#[tokio::test]
async fn a_request_past_the_timeout_yields_four_hundred_and_eight() -> Result<(), Box<dyn StdError>>
{
    let mut settings = support::settings();
    settings.request_timeout = Duration::from_millis(20);
    let router = with_middleware(
        Router::new().route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                StatusCode::OK
            }),
        ),
        support::state(),
        &settings,
    );

    let (status, _) = call(router, Request::get("/slow").body(Body::empty())?).await?;
    assert_eq!(StatusCode::REQUEST_TIMEOUT, status);
    Ok(())
}

#[tokio::test]
async fn a_body_over_the_ceiling_yields_four_hundred_and_thirteen() -> Result<(), Box<dyn StdError>>
{
    let settings = support::settings();
    let oversized = "x".repeat(settings.body_limit + 1);
    let request = Request::post("/health/liveness")
        .header(header::CONTENT_LENGTH, oversized.len())
        .body(Body::from(oversized))?;

    let (status, _) = call(app(), request).await?;
    assert_eq!(StatusCode::PAYLOAD_TOO_LARGE, status);
    Ok(())
}

#[test]
fn the_request_log_carries_the_route_and_never_a_body_or_an_unlisted_query_value() {
    let logs = support::Logs::default();
    let capture = subscriber(Rendering::Json, "info", false, logs.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    tracing::subscriber::with_default(capture, || {
        runtime.block_on(async {
            let state = Arc::new(
                AppState::with_health(Registry::default())
                    .logging_query_parameters(vec![String::from("_count")]),
            );
            let router = ferrobridge_server::router(state, &support::settings());
            let request = Request::get("/health/liveness?_count=10&subject_id=SYNTHETIC-1234")
                .header(request_id::HEADER, "corr-log")
                .body(Body::empty())
                .expect("a request");
            let response = router.oneshot(request).await.expect("an answer");
            assert_eq!(StatusCode::OK, response.status());
        });
    });

    let text = logs.text();
    assert!(text.contains("\"route\":\"/health/liveness\""), "{text}");
    assert!(text.contains("\"method\":\"GET\""), "{text}");
    assert!(text.contains("\"status\":200"), "{text}");
    assert!(text.contains("\"latency_ms\""), "{text}");
    assert!(text.contains("\"request_id\":\"corr-log\""), "{text}");
    assert!(text.contains("_count=10"), "an allowed value is logged");
    assert!(
        !text.contains("SYNTHETIC-1234"),
        "an unlisted query value never reaches the log: {text}"
    );
    assert!(
        !text.contains("subject_id"),
        "an unlisted parameter name is not logged either: {text}"
    );
}

#[test]
fn the_request_log_carries_no_request_body() {
    let logs = support::Logs::default();
    let capture = subscriber(Rendering::Json, "info", false, logs.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    tracing::subscriber::with_default(capture, || {
        runtime.block_on(async {
            let router = with_middleware(
                Router::new().route("/echo", axum::routing::post(|| async { StatusCode::OK })),
                support::state(),
                &support::settings(),
            );
            let request = Request::post("/echo")
                .body(Body::from("SYNTHETIC-BODY-CONTENT"))
                .expect("a request");
            let response = router.oneshot(request).await.expect("an answer");
            assert_eq!(StatusCode::OK, response.status());
        });
    });

    assert!(
        !logs.text().contains("SYNTHETIC-BODY-CONTENT"),
        "a body never reaches the log: {}",
        logs.text()
    );
}

/// Sends `request` through `router` under a capturing JSON subscriber and
/// returns the status with everything the request wrote to the log.
fn logged(router: Router, request: Request<Body>) -> (StatusCode, String) {
    let logs = support::Logs::default();
    let capture = subscriber(Rendering::Json, "info", false, logs.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let status = tracing::subscriber::with_default(capture, || {
        runtime.block_on(async {
            let response = router.oneshot(request).await.expect("an answer");
            response.status()
        })
    });
    (status, logs.text())
}

#[test]
fn a_caught_panic_leaves_its_request_line() {
    let router = with_middleware(
        Router::new().route(
            "/boom",
            get(|| async {
                panic!("the handler gave up");
                #[expect(unreachable_code, reason = "the route panics by design")]
                StatusCode::OK
            }),
        ),
        support::state(),
        &support::settings(),
    );
    let request = Request::get("/boom")
        .header(request_id::HEADER, "corr-panic-log")
        .body(Body::empty())
        .expect("a request");

    let (status, text) = logged(router, request);
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
    assert!(text.contains("\"route\":\"/boom\""), "{text}");
    assert!(text.contains("\"status\":500"), "{text}");
    assert!(text.contains("\"request_id\":\"corr-panic-log\""), "{text}");
    assert_eq!(
        1,
        text.matches("\"message\":\"request\"").count(),
        "one line: {text}"
    );
}

#[test]
fn a_timeout_leaves_its_request_line() {
    let mut settings = support::settings();
    settings.request_timeout = Duration::from_millis(20);
    let router = with_middleware(
        Router::new().route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                StatusCode::OK
            }),
        ),
        support::state(),
        &settings,
    );
    let request = Request::get("/slow")
        .body(Body::empty())
        .expect("a request");

    let (status, text) = logged(router, request);
    assert_eq!(StatusCode::REQUEST_TIMEOUT, status);
    assert!(text.contains("\"route\":\"/slow\""), "{text}");
    assert!(text.contains("\"status\":408"), "{text}");
    assert_eq!(
        1,
        text.matches("\"message\":\"request\"").count(),
        "one line: {text}"
    );
}

#[test]
fn a_refused_oversized_body_leaves_its_request_line() {
    let settings = support::settings();
    let oversized = "x".repeat(settings.body_limit + 1);
    let request = Request::post("/health/liveness")
        .header(header::CONTENT_LENGTH, oversized.len())
        .body(Body::from(oversized))
        .expect("a request");

    let (status, text) = logged(app(), request);
    assert_eq!(StatusCode::PAYLOAD_TOO_LARGE, status);
    assert!(text.contains("\"route\":\"/health/liveness\""), "{text}");
    assert!(text.contains("\"status\":413"), "{text}");
    assert_eq!(
        1,
        text.matches("\"message\":\"request\"").count(),
        "one line: {text}"
    );
}
