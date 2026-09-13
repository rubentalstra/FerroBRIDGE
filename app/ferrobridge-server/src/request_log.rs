// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! One log line per request: the method, the matched route, the status, the
//! latency and the request id.
//!
//! A body never reaches the log, and neither does the raw URL: the matched
//! route is the path template, so an identifier in a path segment stays out.
//! A query value is logged only when the deployment named its parameter in
//! `[telemetry] logged_query_parameters`. No specification governs the log
//! line: our own design, and the clinical-payload rule made mechanical.

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;
use std::time::Instant;

use crate::request_id;
use crate::state::AppState;

/// The longest a logged query value may be.
pub const MAX_VALUE_LENGTH: usize = 64;

/// Logs `request` after it completes and returns its response untouched.
pub async fn log(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let route = request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_owned(),
        |matched| matched.as_str().to_owned(),
    );
    let query = allowed_parameters(
        request.uri().query().unwrap_or_default(),
        state.logged_query_parameters(),
    );
    let id = request_id::of(request.headers())
        .unwrap_or_default()
        .to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    let status = response.status();
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (method, route, query, request_id) =
        (method.as_str(), route.as_str(), query.as_str(), id.as_str());
    let status_code = status.as_u16();
    if status.is_server_error() {
        tracing::error!(
            method,
            route,
            status = status_code,
            latency_ms,
            query,
            request_id,
            "request"
        );
    } else if status.is_client_error() {
        tracing::warn!(
            method,
            route,
            status = status_code,
            latency_ms,
            query,
            request_id,
            "request"
        );
    } else {
        tracing::info!(
            method,
            route,
            status = status_code,
            latency_ms,
            query,
            request_id,
            "request"
        );
    }
    response
}

/// Returns the `key=value` pairs of `query` whose key is in `allowed`.
///
/// The pairs keep the query's own order, separated by a space, and each value
/// is cut at [`MAX_VALUE_LENGTH`] characters. A parameter the deployment did
/// not name contributes nothing, key included.
fn allowed_parameters(query: &str, allowed: &[String]) -> String {
    let mut out = String::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if !allowed.iter().any(|name| name == key) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(key);
        out.push('=');
        out.extend(value.chars().take(MAX_VALUE_LENGTH));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{MAX_VALUE_LENGTH, allowed_parameters};

    #[test]
    fn nothing_is_logged_when_the_allowlist_is_empty() {
        assert_eq!("", allowed_parameters("subject_id=1234&_count=10", &[]));
    }

    #[test]
    fn only_an_allowed_parameter_contributes_and_its_value_is_cut() {
        let allowed = [String::from("_count")];
        assert_eq!(
            "_count=10",
            allowed_parameters("subject_id=1234&_count=10", &allowed)
        );
        let long = "x".repeat(MAX_VALUE_LENGTH + 10);
        assert_eq!(
            format!("_count={}", "x".repeat(MAX_VALUE_LENGTH)),
            allowed_parameters(&format!("_count={long}"), &allowed)
        );
    }

    #[test]
    fn a_pair_without_a_value_and_an_empty_query_contribute_nothing() {
        let allowed = [String::from("_count")];
        assert_eq!("", allowed_parameters("_count", &allowed));
        assert_eq!("", allowed_parameters("", &allowed));
    }
}
