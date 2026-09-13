// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! What every handler shares: the health registry and the log allowlist.

use std::sync::Arc;

use crate::config::Settings;
use crate::health::Registry;
use crate::indicators;

/// The product name the root document reports.
pub const PRODUCT: &str = "FerroBRIDGE";

/// The product version the root document reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The state the router is built over.
#[derive(Debug)]
pub struct AppState {
    /// The indicators readiness runs.
    health: Registry,
    /// The query parameter names whose values may reach the request log.
    logged_query_parameters: Vec<String>,
}

impl AppState {
    /// Returns the state `settings` describes, with one indicator per
    /// configured upstream.
    ///
    /// # Errors
    /// Returns [`crate::config::Error::Client`] when an upstream client
    /// refuses the configuration it was handed.
    pub fn build(settings: &Settings) -> Result<Self, crate::config::Error> {
        let mut indicators: Vec<Arc<dyn crate::health::HealthIndicator>> = Vec::new();
        if let Some(config) = settings.cdr.as_ref() {
            let client =
                ferrobridge_openehr::client::Client::new(config.clone()).map_err(|source| {
                    crate::config::Error::Client {
                        upstream: "cdr",
                        source: Box::new(source),
                    }
                })?;
            indicators.push(Arc::new(indicators::Cdr::new(client)));
        }
        if let Some(config) = settings.terminology.as_ref() {
            let client =
                ferrobridge_term::client::Client::new(config.clone()).map_err(|source| {
                    crate::config::Error::Client {
                        upstream: "terminology",
                        source: Box::new(source),
                    }
                })?;
            indicators.push(Arc::new(indicators::Terminology::new(client)));
        }
        Ok(Self {
            health: Registry::new(indicators),
            logged_query_parameters: settings.telemetry.logged_query_parameters.clone(),
        })
    }

    /// Returns a state with `health` as its registry and no logged query
    /// parameter.
    #[must_use]
    pub fn with_health(health: Registry) -> Self {
        Self {
            health,
            logged_query_parameters: Vec::new(),
        }
    }

    /// Returns this state with `names` as the query parameters it may log.
    #[must_use]
    pub fn logging_query_parameters(mut self, names: Vec<String>) -> Self {
        self.logged_query_parameters = names;
        self
    }

    /// Returns the indicators readiness runs.
    #[must_use]
    pub const fn health(&self) -> &Registry {
        &self.health
    }

    /// Returns the query parameter names whose values may reach the log.
    #[must_use]
    pub fn logged_query_parameters(&self) -> &[String] {
        &self.logged_query_parameters
    }
}
