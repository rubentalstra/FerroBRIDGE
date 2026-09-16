// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Readiness over a stubbed CDR: what the upstream answers decides the status.

use crate::support;
use axum::Router;
use axum::body::Body;
use ferrobridge_server::config::Config;
use ferrobridge_server::state::AppState;
use http::{Request, StatusCode};
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;
use tower::ServiceExt as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Returns the router of a deployment whose CDR is `base`.
fn router(base: &str) -> Result<Router, Box<dyn StdError>> {
    let text = format!("[cdr]\nbase_url = \"{base}/v1\"\ntimeout_ms = 2000\n");
    let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
    let state = Arc::new(AppState::build(&settings)?);
    Ok(ferrobridge_server::router(state, &support::settings()))
}

/// Sends `GET /health/readiness` through `app`.
async fn readiness(app: Router) -> Result<(StatusCode, serde_json::Value), Box<dyn StdError>> {
    let response = app
        .oneshot(Request::get("/health/readiness").body(Body::empty())?)
        .await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

#[tokio::test]
async fn a_cdr_that_answers_makes_readiness_two_hundred() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let (status, document) = readiness(router(&server.uri())?).await?;
    assert_eq!(StatusCode::OK, status);
    assert_eq!(Some("up"), document["state"].as_str());
    assert_eq!(Some("up"), document["indicators"]["cdr"]["state"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_cdr_that_answers_five_hundred_and_three_makes_readiness_five_hundred_and_three()
-> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let (status, document) = readiness(router(&server.uri())?).await?;
    assert_eq!(StatusCode::SERVICE_UNAVAILABLE, status);
    assert_eq!(Some("down"), document["state"].as_str());
    assert_eq!(
        Some("down"),
        document["indicators"]["cdr"]["state"].as_str()
    );
    assert!(
        document["indicators"]["cdr"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("503")),
        "the detail names the upstream status: {document}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_that_refuses_the_credentials_still_counts_as_reachable()
-> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let (status, document) = readiness(router(&server.uri())?).await?;
    assert_eq!(StatusCode::OK, status, "a 401 means the service is there");
    assert_eq!(Some("up"), document["indicators"]["cdr"]["state"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_cdr_that_never_answers_makes_readiness_five_hundred_and_three()
-> Result<(), Box<dyn StdError>> {
    // Port 1 is reserved and unbindable, so the connection is refused at once.
    let (status, document) = readiness(router("http://127.0.0.1:1")?).await?;
    assert_eq!(StatusCode::SERVICE_UNAVAILABLE, status);
    assert_eq!(
        Some("down"),
        document["indicators"]["cdr"]["state"].as_str()
    );
    assert!(
        document["indicators"]["cdr"]["detail"].as_str().is_some(),
        "a down indicator states why: {document}"
    );
    Ok(())
}

#[tokio::test]
async fn liveness_stays_two_hundred_while_an_upstream_is_down() -> Result<(), Box<dyn StdError>> {
    let app = router("http://127.0.0.1:1")?;
    let response = app
        .oneshot(Request::get("/health/liveness").body(Body::empty())?)
        .await?;
    assert_eq!(
        StatusCode::OK,
        response.status(),
        "liveness reports this process, never its upstreams"
    );
    Ok(())
}
