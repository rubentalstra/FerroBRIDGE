// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The mounted surface and the `CapabilityStatement` built from the loaded
//! programs.

use crate::support;
use axum::body::Body;
use ferrobridge_server::facade::programs;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use std::sync::Arc;
use wiremock::MockServer;

use super::FIXTURES;
use super::call;
use super::client;
use super::first_issue;
use super::harness;
use super::raw;

#[tokio::test]
async fn a_disabled_facade_mounts_no_route_and_answers_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let app = ferrobridge_server::router(support::state(), &support::settings());
    let (status, _) = call(app, Request::get("/fhir/metadata").body(Body::empty())?).await?;
    assert_eq!(
        StatusCode::NOT_FOUND,
        status,
        "capability is not authorisation: an unmounted facade is 404, never 403"
    );
    Ok(())
}

#[tokio::test]
async fn the_capability_statement_names_exactly_the_loaded_programs()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let response = raw(
        harness.app(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        Some("application/fhir+json"),
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(Some("CapabilityStatement"), body["resourceType"].as_str());
    assert_eq!(Some("4.0.1"), body["fhirVersion"].as_str());
    let resources = body["rest"][0]["resource"]
        .as_array()
        .expect("the statement names its resource types");
    assert_eq!(1, resources.len(), "{body}");
    let condition = &resources[0];
    assert_eq!(Some("Condition"), condition["type"].as_str());
    assert_eq!(Some(false), condition["updateCreate"].as_bool());
    assert_eq!(Some(true), condition["conditionalCreate"].as_bool());
    assert_eq!(Some(true), condition["conditionalUpdate"].as_bool());
    let interactions: Vec<&str> = condition["interaction"]
        .as_array()
        .expect("the type names its interactions")
        .iter()
        .filter_map(|entry| entry["code"].as_str())
        .collect();
    assert_eq!(vec!["create", "read", "vread", "update"], interactions);
    assert!(
        condition["searchParam"].is_null(),
        "search is not declared until it is implemented: {condition}"
    );
    assert_eq!(Some("validate"), condition["operation"][0]["name"].as_str());
    let system: Vec<&str> = body["rest"][0]["interaction"]
        .as_array()
        .expect("the system level names its interactions")
        .iter()
        .filter_map(|entry| entry["code"].as_str())
        .collect();
    assert_eq!(vec!["transaction"], system);
    Ok(())
}

/// Returns the `(name, definition)` pairs of the system-level operations
/// `body` declares.
fn system_operations(body: &serde_json::Value) -> Vec<(String, String)> {
    body["rest"][0]["operation"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    (
                        entry["name"].as_str().unwrap_or_default().to_owned(),
                        entry["definition"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_template_the_cdr_does_not_hold_names_the_context_and_the_status()
-> Result<(), Box<dyn StdError>> {
    // An unmatched route answers 404 on both definition routes of ITS-REST.
    let cdr = MockServer::start().await;
    let set = programs::read_set(std::path::Path::new(FIXTURES))?;
    let error = programs::fetch_templates(&set, &client(&cdr))
        .await
        .err()
        .ok_or("a template the CDR does not hold refuses the load")?;
    match error {
        programs::LoadError::UnknownTemplate {
            ref template,
            ref context,
            status,
        } => {
            assert_eq!("ferrobridge.diagnose.v1", template);
            assert_eq!("ferrobridge_facade.context", context);
            assert_eq!(StatusCode::NOT_FOUND, status);
        }
        ref other => panic!("the refusal names the context and the status: {other}"),
    }
    Ok(())
}

#[tokio::test]
async fn a_served_operations_lane_is_declared_at_the_system_level() -> Result<(), Box<dyn StdError>>
{
    // R4 CapabilityStatement.rest.operation carries the system-level
    // operations; both FSH definitions of the draft chapter set `system = true`.
    let harness = harness().await;
    let (status, body) = call(
        harness.app_with_operations(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(
        vec![
            (
                String::from("tofhir"),
                String::from("http://fhirconnect.org/fhir/OperationDefinition/ToFhir"),
            ),
            (
                String::from("toopenehr"),
                String::from("http://fhirconnect.org/fhir/OperationDefinition/ToOpenEhr"),
            ),
        ],
        system_operations(&body)
    );
    Ok(())
}

#[tokio::test]
async fn an_absent_operations_lane_is_not_declared() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert!(
        body["rest"][0]["operation"].is_null(),
        "an operations lane that is not served is not declared: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn the_direct_forms_of_the_operations_are_never_declared() -> Result<(), Box<dyn StdError>> {
    // rest-api.adoc (draft), Direct payload invocation: "intentionally not
    // part of the FHIRconnect FHIR Implementation Guide".
    let harness = harness().await;
    let (_, body) = call(
        harness.app_with_operations(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    let rendered = body.to_string();
    assert!(
        !rendered.contains("/tofhir") && !rendered.contains("/toopenehr"),
        "a direct form is declared: {rendered}"
    );
    let names: Vec<String> = system_operations(&body)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        vec![String::from("tofhir"), String::from("toopenehr")],
        names
    );
    Ok(())
}

#[tokio::test]
async fn a_type_with_no_program_is_not_supported_on_the_wire() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/Observation/abc").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        Some("not-supported"),
        first_issue(&body)["code"].as_str(),
        "{body}"
    );
    Ok(())
}

#[test]
fn a_compiler_warning_reaches_the_log_once_as_the_facade_loads() -> Result<(), Box<dyn StdError>> {
    // The KDS project context lists an extension of a model no mapping of it
    // reaches, which the compiler accepts with `fc-unreached-extension`.
    let directory = tempfile::tempdir()?;
    crate::kds::write_mappings(directory.path())?;
    let set = programs::read_set(directory.path())?;
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT)?;
    let index = openehr_mapping_core::index::WebTemplateIndex::build(
        &openehr_mapping_core::template::TemplateSource::Opt14(Box::new(opt)),
    )?;
    let templates = std::collections::BTreeMap::from([(
        String::from(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_TEMPLATE_ID),
        Arc::new(index),
    )]);
    let logs = support::Logs::default();
    let capture = ferrobridge_server::telemetry::subscriber(
        ferrobridge_server::telemetry::Rendering::Json,
        "warn",
        false,
        logs.clone(),
    );
    let compiled =
        tracing::subscriber::with_default(capture, || programs::compile_set(&set, &templates));
    compiled?;
    let text = logs.text();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("fc-unreached-extension"))
        .collect();
    assert_eq!(lines.len(), 1, "one line per warning: {text}");
    let line: serde_json::Value = serde_json::from_str(lines.first().ok_or("one line")?)?;
    assert_eq!(
        Some("ferrobridge_kds_diagnose.context"),
        line["context"].as_str(),
        "{line}"
    );
    Ok(())
}
