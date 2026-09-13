// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FerroBRIDGE server library: the run path the `ferrobridge` binary and
//! its integration tests share.
//!
//! [`cli`] parses the command line, [`config`] reads the file and the
//! environment into one [`config::Settings`], [`telemetry`] installs the
//! subscriber, [`router`] builds the HTTP surface over [`state::AppState`],
//! and [`serve`] runs it on a bound listener until the process is asked to
//! stop. `main.rs` only hands in the arguments and returns the exit code.
#![doc(test(attr(deny(warnings))))]

pub mod cli;
pub mod config;
pub mod health;
pub mod indicators;
pub mod panic;
pub mod request_id;
pub mod request_log;
pub mod state;
pub mod telemetry;

use std::future::Future;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use clap::Parser;
use http::StatusCode;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;

use crate::cli::Cli;
use crate::config::{Config, ServerSettings, Settings};
use crate::state::AppState;

/// The exit code of a job that parses and does not run yet.
///
/// Two is the conventional usage exit, the code a command line uses when the
/// invocation is understood and refused.
pub const EXIT_UNAVAILABLE_JOB: u8 = 2;

/// The exit code of a refused configuration.
///
/// `EX_CONFIG` from `sysexits`
/// (<https://man.freebsd.org/cgi/man.cgi?query=sysexits>), so an orchestrator
/// tells a bad configuration from a failure to serve.
pub const EXIT_CONFIG: u8 = 78;

/// Runs the binary with `args` and returns the process exit code.
///
/// `args` is the whole argument vector, the program name included, so the
/// command-line parser reports usage under the right name.
#[must_use]
#[expect(
    clippy::print_stderr,
    reason = "a usage refusal and a refused configuration are reported before any log subscriber exists"
)]
pub fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => return ExitCode::from(clap_exit(&error)),
    };
    if let Some(issue) = cli.command.pending_issue() {
        eprintln!(
            "ferrobridge: `{}` is not implemented yet; it lands with issue #{issue}",
            cli.command.spelling()
        );
        return ExitCode::from(EXIT_UNAVAILABLE_JOB);
    }
    let settings = match Config::load(cli.config.as_deref()).and_then(|config| config.resolve()) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("ferrobridge: cannot start: {}", chain(&error));
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let stdout_is_terminal = std::io::stdout().is_terminal();
    if let Err(error) = telemetry::init(
        settings.telemetry.format,
        &settings.telemetry.filter,
        stdout_is_terminal,
    ) {
        eprintln!("ferrobridge: cannot start: {}", chain(&error));
        return ExitCode::from(EXIT_CONFIG);
    }
    match serve_command(settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = format!("{error:#}"), "cannot serve");
            ExitCode::FAILURE
        }
    }
}

/// Builds the runtime and serves until the process is asked to stop.
fn serve_command(settings: Settings) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        use anyhow::Context;

        settings.log_lanes();
        let state = Arc::new(AppState::build(&settings).context("building the upstream clients")?);
        tracing::info!(
            version = state::VERSION,
            indicators = state.health().names().join(","),
            "ferrobridge starting"
        );
        let listener = TcpListener::bind(settings.server.listen)
            .await
            .with_context(|| format!("binding {}", settings.server.listen))?;
        tracing::info!(listen = %settings.server.listen, "listening");
        serve(
            listener,
            router(Arc::clone(&state), &settings.server),
            &settings.server,
        )
        .await
        .context("serving HTTP")?;
        tracing::info!("ferrobridge stopped");
        Ok(())
    })
}

/// Returns the exit code a command-line refusal deserves.
///
/// `--help` and `--version` are not failures: clap reports both as an error
/// whose kind says the text was printed
/// (<https://docs.rs/clap/4/clap/error/enum.ErrorKind.html>).
#[expect(
    clippy::print_stdout,
    reason = "clap renders help and version to stdout, which is where a person reads them"
)]
#[expect(
    clippy::print_stderr,
    reason = "a usage refusal is reported before any log subscriber exists"
)]
fn clap_exit(error: &clap::Error) -> u8 {
    if error.use_stderr() {
        eprint!("{error}");
        EXIT_UNAVAILABLE_JOB
    } else {
        print!("{error}");
        0
    }
}

/// Returns `error` and every cause behind it as one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut line = error.to_string();
    let mut cause = error.source();
    while let Some(source) = cause {
        line.push_str(": ");
        line.push_str(&source.to_string());
        cause = source.source();
    }
    line
}

/// Builds the HTTP application over `state`, with the shared middleware.
///
/// `GET /` answers a small JSON document naming the product and its version,
/// `GET /health/liveness` answers `200` while the process is up, and
/// `GET /health/readiness` answers `200` when every registered indicator is up
/// and `503` with each indicator's state otherwise.
pub fn router(state: Arc<AppState>, server: &ServerSettings) -> Router {
    let routes = Router::new()
        .route("/", get(root))
        .route("/health/liveness", get(liveness))
        .route("/health/readiness", get(readiness))
        .with_state(Arc::clone(&state));
    with_middleware(routes, state, server)
}

/// Applies the middleware stack every FerroBRIDGE surface carries to `router`.
///
/// Outermost first: the request-id normalizer, the layer that mints one, the
/// panic renderer, the layer that propagates the id onto the response, the
/// panic catcher, the request timeout, the body-size ceiling, and the request
/// log. The renderer sits outside the propagate layer because it reads the id
/// from the response the propagate layer has just stamped.
pub fn with_middleware(router: Router, state: Arc<AppState>, server: &ServerSettings) -> Router {
    router
        .layer(axum::middleware::from_fn_with_state(
            state,
            request_log::log,
        ))
        .layer(RequestBodyLimitLayer::new(server.body_limit))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            server.request_timeout,
        ))
        .layer(CatchPanicLayer::custom(panic::caught))
        .layer(PropagateRequestIdLayer::new(request_id::HEADER))
        .layer(axum::middleware::map_response(panic::render))
        .layer(SetRequestIdLayer::new(request_id::HEADER, request_id::Mint))
        .layer(axum::middleware::map_request(request_id::strip_illegal))
}

/// `GET /`: the product and the version, as JSON.
async fn root() -> Response {
    axum::Json(serde_json::json!({
        "product": state::PRODUCT,
        "version": state::VERSION,
    }))
    .into_response()
}

/// `GET /health/liveness`: `200` while the process is up.
async fn liveness() -> StatusCode {
    StatusCode::OK
}

/// `GET /health/readiness`: the state of every registered indicator.
async fn readiness(State(state): State<Arc<AppState>>) -> Response {
    let report = state.health().evaluate().await;
    (report.status(), axum::Json(report)).into_response()
}

/// Serves `app` on an already-bound listener until the process receives
/// `SIGTERM` or `SIGINT`, then drains.
///
/// # Errors
/// Returns the I/O error from accepting or serving connections.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    server: &ServerSettings,
) -> std::io::Result<()> {
    serve_until(listener, app, server.shutdown_timeout, shutdown_signal()).await
}

/// Serves `app` on an already-bound listener until `shutdown` completes, then
/// finishes the requests in flight within `drain`.
///
/// A container runtime stops a container with `SIGTERM` to PID 1 and kills it
/// after a grace period, so the drain is bounded here too: a connection still
/// open when `drain` elapses is dropped and the function returns.
///
/// # Errors
/// Returns the I/O error from accepting or serving connections.
pub async fn serve_until<F>(
    listener: TcpListener,
    app: Router,
    drain: Duration,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let signalled = Arc::new(tokio::sync::Notify::new());
    let inner = Arc::clone(&signalled);
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.await;
            inner.notify_one();
        })
        .into_future();
    let mut server = std::pin::pin!(server);
    tokio::select! {
        result = &mut server => return result,
        () = signalled.notified() => {}
    }
    if let Ok(result) = tokio::time::timeout(drain, server).await {
        return result;
    }
    tracing::warn!(
        drain_ms = drain.as_millis(),
        "the drain did not finish in time; the remaining connections are dropped"
    );
    Ok(())
}

/// Completes when the process receives `SIGTERM` or `SIGINT`.
///
/// A failure to install a handler is logged and that arm never completes, so
/// the server keeps serving and the runtime's own kill stays the backstop.
pub async fn shutdown_signal() {
    let interrupt = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {}
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGINT");
                std::future::pending::<()>().await;
            }
        }
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = interrupt => tracing::info!("SIGINT received, draining"),
        () = terminate => tracing::info!("SIGTERM received, draining"),
    }
}
