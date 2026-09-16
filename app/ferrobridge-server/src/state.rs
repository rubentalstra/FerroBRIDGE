// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What every handler shares: the health registry and the log allowlist.

use std::sync::Arc;

use fhirconnect::engine::traverse::Defaults;
use fhirconnect::operations::programs::ProgramSet;

use crate::config::Settings;
use crate::health::Registry;
use crate::indicators;

/// The product name the root document reports.
pub const PRODUCT: &str = "FerroBRIDGE";

/// The product version the root document reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compiled mapping set and what every operation run needs beside it.
///
/// The run timestamp is read per request, because it is the
/// `Provenance.recorded` instant of that run and the composition context start
/// time the engine defaults to; `fhirconnect` reads no clock, so this is where
/// the clock lives.
#[derive(Debug)]
pub struct OperationsLane {
    programs: ProgramSet,
    device: String,
    composer: String,
    language: Option<String>,
    territory: Option<String>,
}

impl OperationsLane {
    /// Pairs a compiled set with the `Device` reference of this deployment.
    #[must_use]
    pub fn new(programs: ProgramSet, device: impl Into<String>) -> Self {
        Self {
            programs,
            device: device.into(),
            composer: String::from(DEFAULT_COMPOSER),
            language: None,
            territory: None,
        }
    }

    /// Returns this lane with the composition defaults an inbound run applies.
    ///
    /// `engine/defaults-for-fields.adoc` puts the composer on the engine and
    /// the composition language and territory on "the project performing the
    /// mapping", so they are configured rather than invented.
    #[must_use]
    pub fn with_composition_defaults(
        mut self,
        composer: impl Into<String>,
        language: Option<String>,
        territory: Option<String>,
    ) -> Self {
        self.composer = composer.into();
        self.language = language;
        self.territory = territory;
        self
    }

    /// Returns the compiled programs one call selects from.
    #[must_use]
    pub const fn programs(&self) -> &ProgramSet {
        &self.programs
    }

    /// Returns the settings of one run, stamped with the current time.
    #[must_use]
    pub fn settings(&self) -> fhirconnect::operations::run::Settings {
        let now = jiff::Timestamp::now().to_string();
        let mut defaults = Defaults::at(now.clone()).with_composer(self.composer.as_str());
        if let Some(ref language) = self.language {
            defaults = defaults.with_language(language.as_str());
        }
        if let Some(ref territory) = self.territory {
            defaults = defaults.with_territory(territory.as_str());
        }
        fhirconnect::operations::run::Settings::new(self.device.as_str(), now)
            .with_defaults(defaults)
    }
}

/// The composer the engine fills in when no mapping does.
///
/// "the `composer` can be defaulted with a value such as `FHIRconnect` party"
/// (`engine/defaults-for-fields.adoc`).
const DEFAULT_COMPOSER: &str = "FHIRconnect";

/// The state the router is built over.
#[derive(Debug)]
pub struct AppState {
    /// The indicators readiness runs.
    health: Registry,
    /// The query parameter names whose values may reach the request log.
    logged_query_parameters: Vec<String>,
    /// The FHIR facade, when its lane is on.
    ///
    /// A disabled facade is `None` and mounts no route, so a request answers
    /// `404` rather than `403` (`docs/architecture.md` §4.6).
    facade: Option<Arc<crate::facade::Facade>>,
    /// The FHIRconnect operations lane, when a mapping set is configured.
    operations: Option<OperationsLane>,
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
        let operations = match settings.mappings {
            Some(ref mappings) if settings.operations.enabled => {
                let programs = crate::mappings::load(mappings).map_err(|source| {
                    crate::config::Error::Client {
                        upstream: "mappings",
                        source: Box::new(source),
                    }
                })?;
                tracing::info!(
                    programs = programs.len(),
                    "the FHIRconnect mapping set is compiled"
                );
                Some(
                    OperationsLane::new(programs, settings.operations.device_reference.as_str())
                        .with_composition_defaults(
                            settings.operations.composer.as_str(),
                            settings.operations.composition_language.clone(),
                            settings.operations.composition_territory.clone(),
                        ),
                )
            }
            _ => None,
        };
        Ok(Self {
            health: Registry::new(indicators),
            logged_query_parameters: settings.telemetry.logged_query_parameters.clone(),
            facade: None,
            operations,
        })
    }

    /// Returns a state with `health` as its registry and no logged query
    /// parameter.
    #[must_use]
    pub fn with_health(health: Registry) -> Self {
        Self {
            health,
            logged_query_parameters: Vec::new(),
            facade: None,
            operations: None,
        }
    }

    /// Returns this state with `facade` mounted and its store probed.
    ///
    /// The identity-store indicator joins readiness with the facade, so a
    /// deployment that does not run the facade does not report on a store it
    /// has no use for.
    #[must_use]
    pub fn with_facade(mut self, facade: Arc<crate::facade::Facade>) -> Self {
        self.health = self.health.and(Arc::new(indicators::IdentityStore::new(
            facade.store_handle(),
        )));
        self.facade = Some(facade);
        self
    }

    /// Returns the facade, when its lane is on.
    #[must_use]
    pub fn facade(&self) -> Option<&Arc<crate::facade::Facade>> {
        self.facade.as_ref()
    }

    /// Returns this state with `names` as the query parameters it may log.
    #[must_use]
    pub fn logging_query_parameters(mut self, names: Vec<String>) -> Self {
        self.logged_query_parameters = names;
        self
    }

    /// Returns this state with `lane` serving the FHIRconnect operations.
    #[must_use]
    pub fn serving_operations(mut self, lane: OperationsLane) -> Self {
        self.operations = Some(lane);
        self
    }

    /// Returns the FHIRconnect operations lane, when one is configured.
    #[must_use]
    pub const fn operations(&self) -> Option<&OperationsLane> {
        self.operations.as_ref()
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
