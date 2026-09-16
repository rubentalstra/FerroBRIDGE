// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Graceful shutdown: a request in flight finishes, and the drain is bounded.

use crate::support;
use axum::Router;
use axum::routing::get;
use ferrobridge_server::{serve_until, with_middleware};
use std::error::Error as StdError;
use std::time::Duration;
use tokio::net::TcpListener;

/// The router the shutdown cases serve: one slow route and one fast one.
fn app() -> Router {
    with_middleware(
        Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    "finished"
                }),
            )
            .route("/fast", get(|| async { "now" })),
        support::state(),
        &support::settings(),
    )
}

#[tokio::test]
async fn a_request_in_flight_finishes_after_the_stop_signal() -> Result<(), Box<dyn StdError>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        serve_until(listener, app(), Duration::from_secs(5), async move {
            if stopped.await.is_err() {
                tracing::debug!("the stop channel closed");
            }
        })
        .await
    });

    let client = reqwest::Client::new();
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.get(format!("http://{address}/slow")).send().await }
    });
    // Give the request time to reach the handler before the signal arrives.
    tokio::time::sleep(Duration::from_millis(50)).await;
    stop.send(()).map_err(|()| "the server is gone")?;

    let response = request.await??;
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!("finished", response.text().await?);
    server.await??;
    Ok(())
}

#[tokio::test]
async fn the_server_returns_once_the_drain_is_done() -> Result<(), Box<dyn StdError>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        serve_until(listener, app(), Duration::from_millis(500), async move {
            if stopped.await.is_err() {
                tracing::debug!("the stop channel closed");
            }
        })
        .await
    });

    let answered = reqwest::Client::new()
        .get(format!("http://{address}/fast"))
        .send()
        .await?;
    assert!(answered.status().is_success());
    stop.send(()).map_err(|()| "the server is gone")?;

    let finished = tokio::time::timeout(Duration::from_secs(5), server).await??;
    finished?;
    Ok(())
}
