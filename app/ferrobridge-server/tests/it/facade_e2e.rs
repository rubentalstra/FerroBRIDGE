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
use ferrobridge_server::cdr::CdrClient;
use ferrobridge_server::cdr::config::CdrConfig;
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
    let client = CdrClient::new(&CdrConfig::new(cdr.base_url().parse()?))?;

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
pub(crate) async fn upload_template(base_url: &str) -> Result<(), Box<dyn StdError>> {
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

/// Uploads the published `KDS_Diagnose` template through the CDR's own route.
async fn upload_kds_template(base_url: &str) -> Result<(), Box<dyn StdError>> {
    let response = reqwest::Client::new()
        .post(format!("{base_url}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(fixtures::KDS_DIAGNOSE_OPT)
        .send()
        .await?;
    assert_eq!(
        StatusCode::CREATED,
        response.status(),
        "the CDR refused the KDS template: {}",
        response.text().await?
    );
    Ok(())
}

/// Returns the entity tag of a `201` without its quotes.
fn entity_tag(response: &reqwest::Response) -> Result<String, Box<dyn StdError>> {
    let raw = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .ok_or("the 201 carries an ETag")?;
    Ok(raw.trim_start_matches("W/").trim_matches('"').to_owned())
}

#[tokio::test]
async fn the_kds_template_admits_the_synthetic_composition_on_a_real_cdr()
-> Result<(), Box<dyn StdError>> {
    // The composition is committed over ITS-REST as canonical JSON and read
    // back; what the CDR sets on commit is the declared set of this leg.
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let cdr = containers::cdr().await?;
    upload_kds_template(cdr.base_url()).await?;
    let index = openehr_mapping_core::index::WebTemplateIndex::build(
        &openehr_mapping_core::template::TemplateSource::opt14(fixtures::KDS_DIAGNOSE_OPT)?,
    )?;
    let built = index.build_from_flat(&crate::kds::flat_composition()?, "2026-09-13T08:00:00Z")?;
    let http = reqwest::Client::new();
    let ehr = http.post(format!("{}/ehr", cdr.base_url())).send().await?;
    assert_eq!(StatusCode::CREATED, ehr.status());
    let ehr_id = entity_tag(&ehr)?;
    let committed = http
        .post(format!("{}/ehr/{ehr_id}/composition", cdr.base_url()))
        .header(header::CONTENT_TYPE, "application/json")
        .body(built.value().to_string())
        .send()
        .await?;
    let status = committed.status();
    let version = entity_tag(&committed).unwrap_or_default();
    assert_eq!(
        StatusCode::CREATED,
        status,
        "the CDR refused the synthetic composition: {}",
        committed.text().await?
    );
    let read = http
        .get(format!(
            "{}/ehr/{ehr_id}/composition/{version}",
            cdr.base_url()
        ))
        .header(header::ACCEPT, "application/json")
        .send()
        .await?;
    assert_eq!(StatusCode::OK, read.status());
    let back = index.accept(read.json::<serde_json::Value>().await?)?;
    let set = ferrobridge_testkit::laws::declared(
        &[],
        &[],
        &serde_json::Value::Object(index.flatten(&built)?),
        &serde_json::Value::Object(index.flatten(&back)?),
    );
    assert_eq!(set["lost"], serde_json::json!([]), "{set}");
    assert_eq!(set["changed"], serde_json::json!([]), "{set}");
    let added: Vec<&str> = set["added"]
        .as_array()
        .ok_or("the declared set lists what the CDR added")?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert!(
        matches!(*added.as_slice(), [only] if only.starts_with("diagnose/_uid = ")),
        "the commit adds the version uid and nothing else: {set}"
    );
    Ok(())
}

#[tokio::test]
async fn the_facade_round_trips_the_kds_condition_through_a_real_cdr()
-> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let cdr = containers::cdr().await?;
    upload_kds_template(cdr.base_url()).await?;
    let client = CdrClient::new(&CdrConfig::new(cdr.base_url().parse()?))?;
    let directory = tempfile::tempdir()?;
    crate::kds::write_mappings(directory.path())?;
    let set = programs::read_set(directory.path())?;
    let templates = programs::fetch_templates(&set, &client).await?;
    let programs = programs::compile_set(&set, &templates)?;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let facade = Arc::new(Facade::new(
        programs,
        store,
        client,
        Settings {
            base_url: String::from("http://ferrobridge.invalid/fhir"),
            ehr_policy: Policy::CreateOnFirstWrite,
            subject_namespace: String::from(SUBJECT_NAMESPACE),
            system_id: String::from("ferrobridge.e2e"),
            language: String::from("de"),
            territory: String::from("DE"),
        },
    ));
    let input: serde_json::Value = serde_json::from_str(fixtures::KDS_DIAGNOSE_CONDITION)?;
    assert_eq!(
        Some(crate::kds::PROFILE),
        input["meta"]["profile"][0].as_str()
    );
    let created = call(
        app(&facade),
        Request::post("/fhir/Condition")
            .header(header::CONTENT_TYPE, "application/fhir+json")
            .body(Body::from(input.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::CREATED, created.0, "{}", created.1);
    let id = created.1["id"]
        .as_str()
        .ok_or("the created resource carries an id")?
        .to_owned();
    let read = call(
        app(&facade),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, read.0, "{}", read.1);
    let mut output = read.1;
    // NOTE: no specification governs this: our own design, the identity the
    // facade assigns and the version it reports are the facade's, not the
    // mapping's, so they leave the comparison with the engine's declared set.
    if let Some(object) = output.as_object_mut() {
        object.remove("id");
        object.remove("meta");
    }
    let mut compared = input.clone();
    if let Some(object) = compared.as_object_mut() {
        object.remove("id");
        object.remove("meta");
    }
    let set = ferrobridge_testkit::laws::declared(&[], &[], &compared, &output);
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/fhirconnect/tests/it/snapshots/it__roundtrip__kds_putget.snap"
    ))?;
    let body = pinned
        .splitn(3, "---")
        .nth(2)
        .ok_or("the snapshot carries a body")?;
    let engine: serde_json::Value = serde_json::from_str(body)?;
    // NOTE: engine/defaults-for-fields.adoc, the composer is defaulted "with a value such
    // as `FHIRconnect`": the engine's law runs on that default, the facade on its own.
    let defaulted = serde_json::json!("recorder.display = \"FHIRconnect\"");
    let configured = serde_json::json!(format!(
        "recorder.display = \"{}\"",
        ferrobridge_server::facade::engine::COMPOSER
    ));
    for list in ["lost", "added", "changed"] {
        let expected: Vec<serde_json::Value> = engine[list]
            .as_array()
            .ok_or("the snapshot carries the list")?
            .iter()
            .filter(|row| {
                row.as_str()
                    .is_some_and(|row| !row.starts_with("id ") && !row.starts_with("meta."))
            })
            .map(|row| {
                if *row == defaulted {
                    configured.clone()
                } else {
                    row.clone()
                }
            })
            .collect();
        assert_eq!(
            set[list].as_array().cloned(),
            Some(expected),
            "the facade and the engine disagree on {list}"
        );
    }
    Ok(())
}

/// Returns the synthetic `Condition` with `id`, coded `code`, for the case's
/// subject.
fn condition_coded(id: &str, code: &str) -> Result<serde_json::Value, Box<dyn StdError>> {
    let mut resource: serde_json::Value = serde_json::from_str(fixtures::R4_CONDITION)?;
    resource["id"] = serde_json::json!(id);
    resource["meta"] = serde_json::json!({ "profile": [PROFILE] });
    resource["subject"] = serde_json::json!({
        "identifier": { "system": SUBJECT_NAMESPACE, "value": SUBJECT_ID }
    });
    resource["code"]["coding"][0]["code"] = serde_json::json!(code);
    Ok(resource)
}

/// Returns how many compositions the CDR holds, over AQL.
async fn compositions(client: &CdrClient) -> Result<usize, Box<dyn StdError>> {
    let request = openehr_its::rest::generated::query::AdhocQueryExecute {
        q: String::from("SELECT c/uid/value AS uid FROM EHR e CONTAINS COMPOSITION c"),
        offset: None,
        fetch: None,
        query_parameters: None,
    };
    match client.query_aql(&request).await?.outcome {
        openehr_its::rest::generated::query::client::QueryExecuteAdhocQueryBodyOutcome::Ok {
            body,
            ..
        } => Ok(body.rows.len()),
        other => Err(format!("the CDR refused the count: {other:?}").into()),
    }
}

#[tokio::test]
async fn a_transaction_commits_once_and_reads_each_entry_back_from_a_real_cdr()
-> Result<(), Box<dyn StdError>> {
    // A committed transaction answers a `transaction-response` Bundle
    // (<https://hl7.org/fhir/R4/http.html#transaction-response>); the re-sent
    // Bundle rule is FerroBRIDGE's own design.
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let cdr = containers::cdr().await?;
    upload_template(cdr.base_url()).await?;
    let client = CdrClient::new(&CdrConfig::new(cdr.base_url().parse()?))?;
    let set = programs::read_set(std::path::Path::new(FIXTURES))?;
    let templates = programs::fetch_templates(&set, &client).await?;
    let programs = programs::compile_set(&set, &templates)?;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let facade = Arc::new(Facade::new(
        programs,
        store,
        client.clone(),
        Settings {
            base_url: String::from("http://ferrobridge.invalid/fhir"),
            ehr_policy: Policy::CreateOnFirstWrite,
            subject_namespace: String::from(SUBJECT_NAMESPACE),
            system_id: String::from("ferrobridge.e2e"),
            language: String::from("en"),
            territory: String::from("GB"),
        },
    ));
    let codes = ["SYN-001", "SYN-002"];
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-first",
              "resource": condition_coded("ferrobridge-e2e-condition-1", codes[0])?,
              "request": { "method": "POST", "url": "Condition" } },
            { "fullUrl": "urn:uuid:0000-second",
              "resource": condition_coded("ferrobridge-e2e-condition-2", codes[1])?,
              "request": { "method": "POST", "url": "Condition" } }
        ]
    });
    let post = |bundle: &serde_json::Value| {
        Request::post("/fhir")
            .header(header::CONTENT_TYPE, "application/fhir+json")
            .body(Body::from(bundle.to_string()))
    };

    let first = call(app(&facade), post(&bundle)?).await?;
    assert_eq!(StatusCode::OK, first.0, "{}", first.1);
    assert_eq!(
        Some("transaction-response"),
        first.1["type"].as_str(),
        "{}",
        first.1
    );
    let locations: Vec<String> = (0..2)
        .map(|index| {
            first.1["entry"][index]["response"]["location"]
                .as_str()
                .map(str::to_owned)
                .ok_or(format!("entry {index} answers a location: {}", first.1))
        })
        .collect::<Result<_, _>>()?;
    assert_ne!(locations[0], locations[1], "two entries, two resources");
    assert_eq!(2, compositions(&client).await?);

    for (location, code) in locations.iter().zip(codes) {
        let path = location
            .strip_prefix("http://ferrobridge.invalid")
            .and_then(|rest| rest.split("/_history/").next())
            .ok_or(format!("{location} is under the facade base"))?;
        let read = call(app(&facade), Request::get(path).body(Body::empty())?).await?;
        assert_eq!(StatusCode::OK, read.0, "{}", read.1);
        assert_eq!(
            Some(code),
            read.1["code"]["coding"][0]["code"].as_str(),
            "{path} answers the entry it came from: {}",
            read.1
        );
    }

    let second = call(app(&facade), post(&bundle)?).await?;
    assert_eq!(StatusCode::OK, second.0, "{}", second.1);
    let resent: Vec<Option<&str>> = (0..2)
        .map(|index| second.1["entry"][index]["response"]["location"].as_str())
        .collect();
    let expected: Vec<Option<&str>> = locations.iter().map(|at| Some(at.as_str())).collect();
    assert_eq!(
        expected, resent,
        "the re-sent Bundle names the resources its first delivery created"
    );
    assert_eq!(
        2,
        compositions(&client).await?,
        "the re-sent Bundle commits no third composition"
    );
    Ok(())
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
