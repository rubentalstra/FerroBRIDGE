// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The facade against a real CDR, behind the `FERROBRIDGE_E2E` gate.
//!
//! The `wiremock` suite beside this one pins every status and every header;
//! this case proves the round trip itself: a synthetic `Condition` commits as
//! a composition of the diagnosis template and reads back through the same
//! program, with the identity the map assigned.

use axum::Router;
use axum::body::Body;
use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::config::Config;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::Settings;
use ferrobridge_server::facade::ehr::Policy;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::identity::store::Store;
use ferrobridge_server::facade::programs;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
use ferrobridge_testkit::containers;
use ferrobridge_testkit::fixtures;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use std::sync::Arc;
use tower::ServiceExt as _;

/// The fixture directory the case loads its mapping set from.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/fixtures");

/// The profile the fixture context claims.
const PROFILE: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-facade-diagnosis";

/// The subject the synthetic `Condition` names.
const SUBJECT_ID: &str = "synthetic-subject-0001";

/// The namespace that subject belongs to.
const SUBJECT_NAMESPACE: &str = "http://example.org/fhir/sid/ferrobridge-subject";

#[tokio::test]
async fn the_facade_commits_a_condition_and_reads_it_back_from_a_real_cdr()
-> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let cdr = containers::cdr().await?;
    upload_template(cdr.base_url()).await?;
    let client = Client::new(Config::new(cdr.base_url().parse()?))?;

    let set = programs::read_set(std::path::Path::new(FIXTURES))?;
    let templates = programs::fetch_templates(&set, &client).await?;
    let programs = programs::compile_set(&set, &templates)?;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let facade = Arc::new(Facade::new(
        programs,
        store,
        client,
        Settings {
            base_url: String::from("http://ferrobridge.invalid/fhir"),
            // The CDR holds no EHR for the synthetic subject, so this case
            // exercises the policy that creates one on the first write.
            ehr_policy: Policy::CreateOnFirstWrite,
            subject_namespace: String::from(SUBJECT_NAMESPACE),
            system_id: String::from("ferrobridge.e2e"),
            language: String::from("en"),
            territory: String::from("GB"),
        },
    ));

    let created = call(app(&facade), post_condition()?).await?;
    assert_eq!(StatusCode::CREATED, created.0, "{}", created.1);
    let id = created.1["id"]
        .as_str()
        .ok_or("the created resource carries an id")?
        .to_owned();
    assert_eq!(Some("1"), created.1["meta"]["versionId"].as_str());

    let read = call(
        app(&facade),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, read.0, "{}", read.1);
    assert_eq!(
        Some(id.as_str()),
        read.1["id"].as_str(),
        "the read answers under the id the create assigned"
    );
    assert_eq!(
        created.1["code"], read.1["code"],
        "the diagnosis name did not survive the round trip"
    );
    assert!(
        read.1["meta"]["source"]
            .as_str()
            .is_some_and(|source| source.contains("/composition/")),
        "meta.source names the composition version: {}",
        read.1
    );
    Ok(())
}

/// Returns the application under test.
fn app(facade: &Arc<Facade>) -> Router {
    let state = AppState::with_health(Registry::default()).with_facade(Arc::clone(facade));
    ferrobridge_server::router(
        Arc::new(state),
        &ferrobridge_server::config::ServerSettings {
            listen: "127.0.0.1:0".parse().expect("a socket address"),
            request_timeout: core::time::Duration::from_secs(30),
            shutdown_timeout: core::time::Duration::from_secs(5),
            body_limit: 1024 * 1024,
        },
    )
}

/// Uploads the synthetic diagnosis template through the CDR's own route.
async fn upload_template(base_url: &str) -> Result<(), Box<dyn StdError>> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(fixtures::DIAGNOSE_OPT)
        .send()
        .await?;
    assert_eq!(
        StatusCode::CREATED,
        response.status(),
        "the CDR refused the synthetic template: {}",
        response.text().await?
    );
    Ok(())
}

/// Returns the create request the case sends.
fn post_condition() -> Result<Request<Body>, Box<dyn StdError>> {
    let mut resource: serde_json::Value = serde_json::from_str(fixtures::R4_CONDITION)?;
    resource["meta"] = serde_json::json!({ "profile": [PROFILE] });
    resource["subject"] = serde_json::json!({
        "identifier": { "system": SUBJECT_NAMESPACE, "value": SUBJECT_ID }
    });
    Ok(Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(resource.to_string()))?)
}

/// Sends `request` through `app` and reads the status and the body.
async fn call(
    app: Router,
    request: Request<Body>,
) -> Result<(StatusCode, serde_json::Value), Box<dyn StdError>> {
    let response = app.oneshot(request).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_unparsed| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    Ok((status, body))
}
