// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Readiness over a registry of per-subsystem indicators.
//!
//! An indicator is a named, bounded check of one upstream. Liveness answers
//! `200` while the process is up; readiness runs every registered indicator
//! and answers `503` while any of them is down, with a body naming each one.
//! No specification governs health probes: our own design.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// The upper bound on one indicator check.
///
/// A wedged upstream cannot hang a probe: a check that runs past this is
/// reported down with that as its detail.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// The state of one indicator, or of the aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The subsystem answered.
    Up,
    /// The subsystem did not answer.
    Down,
}

/// The outcome of one indicator check.
#[derive(Debug, Clone, Serialize)]
pub struct IndicatorState {
    /// Whether the subsystem answered.
    pub state: State,
    /// Why it did not, when it did not. Never a body and never a credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl IndicatorState {
    /// Returns the state of a subsystem that answered.
    #[must_use]
    pub const fn up() -> Self {
        Self {
            state: State::Up,
            detail: None,
        }
    }

    /// Returns the state of a subsystem that did not answer, with the reason.
    #[must_use]
    pub fn down(detail: impl Into<String>) -> Self {
        Self {
            state: State::Down,
            detail: Some(detail.into()),
        }
    }
}

/// The future one check returns.
///
/// The registry holds indicators behind `dyn`, which an `async fn` in a trait
/// is not, so a check hands back a boxed future
/// (<https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility>).
pub type Check<'a> = Pin<Box<dyn Future<Output = IndicatorState> + Send + 'a>>;

/// One named, bounded check of one subsystem.
pub trait HealthIndicator: fmt::Debug + Send + Sync {
    /// Returns the name this indicator answers under in the readiness body.
    fn name(&self) -> &'static str;

    /// Runs the check.
    ///
    /// The registry bounds it to [`CHECK_TIMEOUT`], so an implementation needs
    /// no timer of its own beyond its client's.
    fn check(&self) -> Check<'_>;
}

/// The indicators readiness evaluates.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// The indicators, in registration order.
    indicators: Arc<[Arc<dyn HealthIndicator>]>,
}

impl Registry {
    /// Returns a registry over `indicators`.
    #[must_use]
    pub fn new(indicators: Vec<Arc<dyn HealthIndicator>>) -> Self {
        Self {
            indicators: indicators.into(),
        }
    }

    /// Returns this registry with `indicator` registered after the others.
    #[must_use]
    pub fn and(self, indicator: Arc<dyn HealthIndicator>) -> Self {
        let mut indicators: Vec<Arc<dyn HealthIndicator>> = self.indicators.to_vec();
        indicators.push(indicator);
        Self::new(indicators)
    }

    /// Returns the names of the registered indicators, in registration order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.indicators
            .iter()
            .map(|indicator| indicator.name())
            .collect()
    }

    /// Runs every indicator and folds the results into one report.
    ///
    /// The checks run on the request, one after another, each bounded to
    /// [`CHECK_TIMEOUT`]. A background loop would answer from a cache and call
    /// a dead upstream healthy until the next tick, so the probe asks.
    pub async fn evaluate(&self) -> Report {
        let mut indicators = BTreeMap::new();
        let mut state = State::Up;
        for indicator in self.indicators.iter() {
            let outcome = match tokio::time::timeout(CHECK_TIMEOUT, indicator.check()).await {
                Ok(outcome) => outcome,
                Err(_) => IndicatorState::down(format!(
                    "the check did not answer inside {} ms",
                    CHECK_TIMEOUT.as_millis()
                )),
            };
            if outcome.state == State::Down {
                state = State::Down;
            }
            indicators.insert(indicator.name(), outcome);
        }
        Report { state, indicators }
    }
}

/// What readiness answers with.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The aggregate: down while any indicator is down.
    pub state: State,
    /// Every indicator by name, in name order.
    pub indicators: BTreeMap<&'static str, IndicatorState>,
}

impl Report {
    /// Returns the HTTP status this report answers with.
    #[must_use]
    pub const fn status(&self) -> http::StatusCode {
        match self.state {
            State::Up => http::StatusCode::OK,
            State::Down => http::StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Check, HealthIndicator, IndicatorState, Registry, State};
    use http::StatusCode;
    use std::sync::Arc;

    #[derive(Debug)]
    struct Fixed(&'static str, State);

    impl HealthIndicator for Fixed {
        fn name(&self) -> &'static str {
            self.0
        }

        fn check(&self) -> Check<'_> {
            let state = self.1;
            Box::pin(async move {
                match state {
                    State::Up => IndicatorState::up(),
                    State::Down => IndicatorState::down("the stub is down"),
                }
            })
        }
    }

    #[tokio::test]
    async fn an_empty_registry_is_up() {
        let report = Registry::default().evaluate().await;
        assert_eq!(State::Up, report.state);
        assert_eq!(StatusCode::OK, report.status());
        assert!(report.indicators.is_empty());
    }

    #[tokio::test]
    async fn one_down_indicator_takes_the_aggregate_down_and_every_state_is_reported() {
        let registry = Registry::new(vec![
            Arc::new(Fixed("cdr", State::Down)),
            Arc::new(Fixed("terminology", State::Up)),
        ]);
        let report = registry.evaluate().await;
        assert_eq!(State::Down, report.state);
        assert_eq!(StatusCode::SERVICE_UNAVAILABLE, report.status());
        assert_eq!(2, report.indicators.len());
        assert_eq!(
            Some(State::Down),
            report.indicators.get("cdr").map(|state| state.state)
        );
        assert_eq!(
            Some(State::Up),
            report
                .indicators
                .get("terminology")
                .map(|state| state.state)
        );
        assert_eq!(vec!["cdr", "terminology"], registry.names());
    }
}
