// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first passing cases of the facade corpus against the reference CDR,
//! behind the `FERROBRIDGE_E2E` gate.
//!
//! The sample is the first [`SAMPLE`] ids of the committed pass list, so the
//! run stays short and follows the list as it grows. Each is created and read
//! through a facade whose client reaches the real CDR, and holds when the
//! read is an R4 instance and `PutGet` changes nothing the example carried.

use std::error::Error as StdError;
use std::sync::Arc;

use axum::body::Body;
use ferrobridge_server::cdr::CdrClient;
use ferrobridge_server::cdr::config::CdrConfig;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::Settings;
use ferrobridge_server::facade::ehr::Policy;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::programs::Programs;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
use ferrobridge_testkit::conformance::Corpus;
use ferrobridge_testkit::containers;
use ferrobridge_testkit::examples::Package;
use http::Request;
use http::StatusCode;
use http::header;

use super::Context;
use super::claiming;
use super::compared;
use super::compile;
use super::invalid;
use crate::facade::call;
use crate::facade::handle;
use crate::facade::server_settings;

/// How many passing cases the gated run takes from the pass list.
const SAMPLE: usize = 3;

/// The namespace the reference CDR's EHRs are created under.
const SUBJECT_NAMESPACE: &str = "http://example.org/fhir/sid/ferrobridge-subject";

/// Returns the first [`SAMPLE`] ids the committed pass list records.
fn sample() -> Result<Vec<String>, Box<dyn StdError>> {
    let text = std::fs::read_to_string(Corpus::FhirR4Facade.pass_list())?;
    Ok(text
        .lines()
        .filter(|line| !line.starts_with("total "))
        .take(SAMPLE)
        .map(str::to_owned)
        .collect())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_passing_facade_examples_round_trip_through_a_real_cdr()
-> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() || !super::fetched() {
        return Ok(());
    }
    let cases = sample()?;
    let cdr = containers::cdr().await?;
    crate::facade_e2e::upload_template(cdr.base_url()).await?;
    crate::facade_e2e::upload_kds_template(cdr.base_url()).await?;
    for id in cases {
        let (context_id, file) = id.split_once('/').ok_or("a case id names its context")?;
        let context = Context::ALL
            .into_iter()
            .find(|context| context.id() == context_id)
            .ok_or("a case id names a context the corpus runs")?;
        let compiled = compile(context).await?;
        let facade = Arc::new(Facade::new(
            Programs::new(compiled.loaded.clone()),
            handle(&Arc::new(MemoryStore::new())),
            CdrClient::new(&CdrConfig::new(cdr.base_url().parse()?))?,
            Settings {
                ehr_policy: Policy::CreateOnFirstWrite,
                subject_namespace: String::from(SUBJECT_NAMESPACE),
                system_id: String::from("ferrobridge.e2e"),
                ..context.settings()
            },
        ));
        let state = AppState::with_health(Registry::default()).with_facade(facade);
        let app = ferrobridge_server::router(Arc::new(state), &server_settings());
        let resource: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
            Package::R4.directory().join(file),
        )?)?;
        let resource_type = resource["resourceType"]
            .as_str()
            .ok_or("an example names its type")?;
        let (status, created) = call(
            app.clone(),
            Request::post(format!("/fhir/{resource_type}"))
                .header(header::CONTENT_TYPE, "application/fhir+json")
                .body(Body::from(
                    claiming(&resource, context.profile()).to_string(),
                ))?,
        )
        .await?;
        assert_eq!(StatusCode::CREATED, status, "{id}: {created}");
        let created_id = created["id"]
            .as_str()
            .ok_or("the created resource carries an id")?;
        let (status, read) = call(
            app,
            Request::get(format!("/fhir/{resource_type}/{created_id}")).body(Body::empty())?,
        )
        .await?;
        assert_eq!(StatusCode::OK, status, "{id}: {read}");
        assert_eq!(None, invalid(&read), "{id}");
        let putget =
            ferrobridge_testkit::laws::declared(&[], &[], &compared(&resource), &compared(&read));
        assert_eq!(serde_json::json!([]), putget["changed"], "{id}: {putget}");
    }
    Ok(())
}
