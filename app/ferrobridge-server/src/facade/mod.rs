// SPDX-FileCopyrightText: Vernum Projecten B.V.
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
pub mod ingest;
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

use crate::cdr::CdrClient;
use crate::facade::identity::claims::Claims;
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
    /// The `COMPOSITION.language` the facade writes, an ISO 639-1 code.
    ///
    /// `engine/defaults-for-fields.adoc` puts the composer and the context
    /// start time on the engine and every other field on "the project
    /// performing the mapping", so this deployment states it. There is no
    /// correct default for the language of a clinical record, so the
    /// configuration has none.
    pub language: String,
    /// The `COMPOSITION.territory` the facade writes, an ISO 3166-1 code.
    pub territory: String,
}

/// Everything a facade handler reaches.
#[derive(Debug)]
pub struct Facade {
    /// The compiled programs this deployment loaded.
    programs: Programs,
    /// The identity map.
    store: Arc<dyn Store>,
    /// The source keys the in-flight deliveries into that map hold.
    claims: Claims,
    /// The CDR client.
    client: CdrClient,
    /// What the deployment configured.
    settings: Settings,
}

impl Facade {
    /// Returns a facade over `programs`, `store` and `client`.
    #[must_use]
    pub const fn new(
        programs: Programs,
        store: Arc<dyn Store>,
        client: CdrClient,
        settings: Settings,
    ) -> Self {
        Self {
            programs,
            store,
            claims: Claims::new(),
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
    pub const fn client(&self) -> &CdrClient {
        &self.client
    }

    /// Returns the CDR client one request calls through.
    ///
    /// The inbound `X-Request-Id` travels onto every outbound call, so one
    /// correlation identifier covers the request and the CDR calls it makes.
    #[must_use]
    pub fn client_for(&self, headers: &http::HeaderMap) -> CdrClient {
        headers
            .get(crate::request_id::HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| crate::cdr::ids::RequestId::new(text).ok())
            .map_or_else(|| self.client.clone(), |id| self.client.with_request_id(id))
    }

    /// Returns what the deployment configured.
    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Returns the ingest service over this facade, calling the CDR through
    /// `client`.
    ///
    /// A handler passes [`Facade::client_for`] so one request's CDR calls
    /// carry its request id; another face passes [`Facade::client`] or a
    /// client of its own.
    #[must_use]
    pub fn ingest(&self, client: CdrClient) -> ingest::Ingest<'_> {
        self.ingest_under(client, &self.settings)
    }

    /// Returns the ingest service over this facade's programs, identity map
    /// and claims, calling the CDR through `client` under `settings`.
    ///
    /// A face whose deployment states its own EHR policy passes settings of
    /// its own; the identity map and the claims stay the facade's, so one
    /// source delivered through two faces is recognised by both.
    #[must_use]
    pub fn ingest_under<'a>(
        &'a self,
        client: CdrClient,
        settings: &'a Settings,
    ) -> ingest::Ingest<'a> {
        ingest::Ingest::new(
            &self.programs,
            self.store.as_ref(),
            &self.claims,
            client,
            settings,
        )
    }
}

/// Returns the facade's routes, mounted under [`BASE_PATH`].
///
/// Only what this milestone implements is mounted: the conformance statement,
/// a type-level create, `$validate`, an instance read and update, and a
/// system-level transaction. Search and batch have no route, so a request for
/// either answers `404` (<https://hl7.org/fhir/R4/http.html>). `operations`
/// is whether the same router serves the FHIRconnect operations, which the
/// conformance statement declares only then, and `hl7v2` whether the HL7 v2
/// face runs, which the statement's implementation description names only
/// then.
pub fn routes(
    facade: Arc<Facade>,
    operations: capability::Operations,
    hl7v2: capability::Hl7v2,
) -> Router {
    Router::new()
        .route(
            &format!("{BASE_PATH}/metadata"),
            get(
                move |axum::extract::State(facade): axum::extract::State<Arc<Facade>>,
                      headers: http::HeaderMap,
                      uri: http::Uri| async move {
                    handlers::metadata_route(&facade, &headers, &uri, operations, hl7v2)
                },
            ),
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
