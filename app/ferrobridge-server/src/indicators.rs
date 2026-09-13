// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The indicators readiness runs: one per configured upstream.
//!
//! Each probe asks its upstream whether it answers at all. A status counts as
//! reachable, `401` and `404` included, because the question is whether the
//! service is there; a `5xx` and a failure to connect count as down. No
//! specification governs the probe: our own design.

use http::StatusCode;

use crate::health::{Check, HealthIndicator, IndicatorState};

// TODO(#91): the CDM indicator, once the writer owns the database connection.

/// The openEHR CDR probe.
#[derive(Debug, Clone)]
pub struct Cdr {
    /// The client the probe calls through, with its configured credentials.
    client: ferrobridge_openehr::client::Client,
}

impl Cdr {
    /// Returns a probe over `client`.
    #[must_use]
    pub const fn new(client: ferrobridge_openehr::client::Client) -> Self {
        Self { client }
    }
}

impl HealthIndicator for Cdr {
    fn name(&self) -> &'static str {
        "cdr"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move { classify(self.client.reachability().await.map_err(reason)) })
    }
}

/// The FHIR terminology server probe.
#[derive(Debug, Clone)]
pub struct Terminology {
    /// The client the probe calls through, with its configured credentials.
    client: ferrobridge_term::client::Client,
}

impl Terminology {
    /// Returns a probe over `client`.
    #[must_use]
    pub const fn new(client: ferrobridge_term::client::Client) -> Self {
        Self { client }
    }
}

impl HealthIndicator for Terminology {
    fn name(&self) -> &'static str {
        "terminology"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move { classify(self.client.reachability().await.map_err(reason)) })
    }
}

/// Returns `error` and every cause behind it as one line.
///
/// The readiness body states why an upstream is down, so the whole chain is
/// rendered (RFC 0201, `Error::source`); the typed error itself stays in the
/// client and never becomes a string on any other path.
fn reason(error: impl std::error::Error) -> String {
    let mut line = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(source) = cause {
        line.push_str(": ");
        line.push_str(&source.to_string());
        cause = source.source();
    }
    line
}

/// Returns the indicator state a probe outcome means.
fn classify(outcome: Result<StatusCode, String>) -> IndicatorState {
    match outcome {
        Err(detail) => IndicatorState::down(detail),
        Ok(status) if status.is_server_error() => {
            IndicatorState::down(format!("the upstream answered {status}"))
        }
        Ok(_) => IndicatorState::up(),
    }
}

#[cfg(test)]
mod tests {
    use super::classify;
    use crate::health::State;
    use http::StatusCode;

    #[test]
    fn a_2xx_and_a_4xx_are_up_and_a_5xx_is_down() {
        for reachable in [
            StatusCode::OK,
            StatusCode::UNAUTHORIZED,
            StatusCode::NOT_FOUND,
        ] {
            assert_eq!(State::Up, classify(Ok(reachable)).state, "{reachable}");
        }
        for unreachable in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert_eq!(
                State::Down,
                classify(Ok(unreachable)).state,
                "{unreachable}"
            );
        }
    }

    #[test]
    fn a_call_that_never_reached_the_upstream_is_down_and_keeps_its_reason() {
        let outcome = classify(Err(String::from("the request could not be sent")));
        assert_eq!(State::Down, outcome.state);
        assert_eq!(
            Some("the request could not be sent"),
            outcome.detail.as_deref()
        );
    }
}
