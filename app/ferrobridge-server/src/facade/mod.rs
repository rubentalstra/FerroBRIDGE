// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR R4 REST facade over the openEHR CDR.
//!
//! FerroBRIDGE stores no clinical data of its own: every interaction here maps
//! onto CDR operations through a compiled FHIRconnect program
//! (`docs/architecture.md` §4.6). What the facade keeps is identity, in the
//! store of [`identity`], so a re-sent resource updates the composition it
//! already produced and a read resolves a FHIR id back to one entry.
//!
//! Two rules run through every module. A status is a `StatusCode`, mapped from
//! the CDR's own outcome by the one table in [`status`]. Everything the facade
//! authors is an `OperationOutcome`, and an upstream openEHR error body
//! travels inside `issue.diagnostics` rather than reaching the wire raw.
//!
//! A disabled facade mounts no route, so a request answers `404` rather than
//! `403`: capability is not authorisation.

pub mod capability;
pub mod commit;
pub mod ehr;
pub mod engine;
pub mod handlers;
pub mod identity;
pub mod media;
pub mod outcome;
pub mod programs;
pub mod reply;
pub mod request;
pub mod status;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use axum::routing::post;
use ferrobridge_openehr::client::Client;

use crate::facade::identity::store::Store;
use crate::facade::programs::Programs;

/// The path every facade route sits under.
///
/// The FHIR service base is `[base]` of the R4 HTTP chapter
/// (<https://hl7.org/fhir/R4/http.html>), and this deployment mounts it at
/// `/fhir` so the health routes and the FHIRconnect operations keep their own
/// space (no specification governs the mount point: our own design).
pub const BASE_PATH: &str = "/fhir";

/// What one facade deployment was configured with.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The absolute FHIR service base, as a client sees it.
    ///
    /// `Location` is written under this.
    pub base_url: String,
    /// Whether an unknown subject gets an EHR on its first write.
    pub ehr_policy: ehr::Policy,
    /// The namespace a literal subject reference is read in.
    pub subject_namespace: String,
    /// The `AUDIT_DETAILS.system_id` every commit records.
    pub system_id: String,
}

/// Everything a facade handler reaches.
#[derive(Debug)]
pub struct Facade {
    /// The compiled programs this deployment loaded.
    programs: Programs,
    /// The identity map.
    store: Arc<dyn Store>,
    /// The CDR client.
    client: Client,
    /// What the deployment configured.
    settings: Settings,
}

impl Facade {
    /// Returns a facade over `programs`, `store` and `client`.
    #[must_use]
    pub const fn new(
        programs: Programs,
        store: Arc<dyn Store>,
        client: Client,
        settings: Settings,
    ) -> Self {
        Self {
            programs,
            store,
            client,
            settings,
        }
    }

    /// Returns the compiled programs.
    #[must_use]
    pub const fn programs(&self) -> &Programs {
        &self.programs
    }

    /// Returns the identity map.
    #[must_use]
    pub fn store(&self) -> &dyn Store {
        self.store.as_ref()
    }

    /// Returns a handle on the identity map, for the readiness probe.
    #[must_use]
    pub fn store_handle(&self) -> Arc<dyn Store> {
        Arc::clone(&self.store)
    }

    /// Returns the CDR client.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// Returns the CDR client one request calls through.
    ///
    /// The inbound `X-Request-Id` travels onto every outbound call, so one
    /// correlation identifier covers the request and the CDR calls it makes.
    #[must_use]
    pub fn client_for(&self, headers: &http::HeaderMap) -> Client {
        headers
            .get(crate::request_id::HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| ferrobridge_openehr::ids::RequestId::new(text).ok())
            .map_or_else(|| self.client.clone(), |id| self.client.with_request_id(id))
    }

    /// Returns what the deployment configured.
    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }
}

/// Returns the facade's routes, mounted under [`BASE_PATH`].
///
/// Only what this milestone implements is mounted: the conformance statement,
/// a type-level create, `$validate`, an instance read and update, and a
/// system-level transaction. Search and batch have no route, so a request for
/// either answers `404` (<https://hl7.org/fhir/R4/http.html>).
pub fn routes(facade: Arc<Facade>) -> Router {
    Router::new()
        .route(
            &format!("{BASE_PATH}/metadata"),
            get(handlers::metadata_route),
        )
        .route(BASE_PATH, post(handlers::transaction_route))
        .route(
            &format!("{BASE_PATH}/{{resource_type}}"),
            post(handlers::create_route),
        )
        .route(
            &format!("{BASE_PATH}/{{resource_type}}/$validate"),
            post(handlers::validate_route),
        )
        .route(
            &format!("{BASE_PATH}/{{resource_type}}/{{id}}"),
            get(handlers::read_route).put(handlers::update_route),
        )
        .with_state(facade)
}
