// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the two operations answer, enveloped and direct, and how the body
//! and the query pin the template.

use axum::body::Body;
use http::{Request, StatusCode, header};
use std::error::Error as StdError;

use super::FHIR_JSON;
use super::OPENEHR_JSON;
use super::app;
use super::bundle;
use super::call;
use super::composition;
use super::composition_text;
use super::mapping_tree;

/// The template the synthetic mapping set compiles against.
const TEMPLATE: &str = "ferrobridge.diagnose.v1";

#[tokio::test]
async fn toopenehr_answers_a_parameters_with_the_composition() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("Parameters"), answer["resourceType"].as_str());
    assert_eq!(Some("composition"), answer["parameter"][0]["name"].as_str());
    assert!(
        answer["parameter"][0]["valueString"]
            .as_str()
            .is_some_and(|text| text.contains("Synthetic problem one")),
        "the composition carries the mapped problem name"
    );
    Ok(())
}

#[tokio::test]
async fn tofhir_answers_a_bundle_with_a_provenance() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": built.to_string() }],
    });
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("Bundle"), answer["resourceType"].as_str());
    assert_eq!(Some("collection"), answer["type"].as_str());
    let kinds: Vec<&str> = answer["entry"]
        .as_array()
        .ok_or("the Bundle carries entries")?
        .iter()
        .filter_map(|entry| entry["resource"]["resourceType"].as_str())
        .collect();
    assert!(
        kinds.contains(&"Condition") && kinds.contains(&"Provenance"),
        "the Bundle carries the mapped resource and the Provenance: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "Provenance").count(),
        1,
        "every run carries exactly one Provenance"
    );
    Ok(())
}

#[tokio::test]
async fn the_direct_form_reads_an_openehr_json_body() -> Result<(), Box<dyn StdError>> {
    // "POST [base]/tofhir, Content-Type: application/openehr+json" is the
    // chapter's direct payload invocation, which it keeps outside the FHIR
    // implementation guide.
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/tofhir")
            .header(header::CONTENT_TYPE, OPENEHR_JSON)
            .body(Body::from(built.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("Bundle"), answer["resourceType"].as_str());
    Ok(())
}

#[tokio::test]
async fn the_direct_form_answers_the_composition_itself() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(OPENEHR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("COMPOSITION"), answer["_type"].as_str());
    Ok(())
}

#[tokio::test]
async fn the_direct_form_answers_the_requested_format() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/toopenehr?format=flat")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(OPENEHR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert!(
        answer["_type"].is_null(),
        "a flat document carries no _type: {body}"
    );
    assert!(
        answer
            .as_object()
            .is_some_and(|members| members.keys().any(|key| key.contains('/'))),
        "a flat document keys its values by template path"
    );
    Ok(())
}

#[tokio::test]
async fn the_body_takes_precedence_over_the_query() -> Result<(), Box<dyn StdError>> {
    // "Where the same field is supplied both in the body and as a query
    // parameter, the body takes precedence" (rest-api.adoc, draft, section
    // Query parameters).
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            { "name": "composition", "valueString": built.to_string() },
            { "name": "templateId", "valueString": TEMPLATE },
        ],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir?templateId=not.loaded.v1")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(
        StatusCode::OK,
        status,
        "the body's templateId wins over the query's: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn the_query_pins_the_template_when_the_body_does_not() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let flat = call(
        app(&root)?,
        Request::post("/fhir/toopenehr?format=flat")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, flat.0, "{}", flat.2);
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": flat.2 }],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir?templateId=not.loaded.v1")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(
        StatusCode::BAD_REQUEST,
        status,
        "the query's templateId reached the run: {body}"
    );
    assert!(
        body.contains("not.loaded.v1"),
        "the outcome names the template the query pinned: {body}"
    );
    drop(built);
    Ok(())
}

#[tokio::test]
async fn the_enveloped_toopenehr_reads_a_parameters_body() -> Result<(), Box<dyn StdError>> {
    // ToOpenEhr.fsh declares `bundle` as an `in` parameter while the chapter's
    // prose puts the Bundle in the body directly; both forms are read.
    let root = mapping_tree()?;
    let carried: serde_json::Value = serde_json::from_str(&bundle()?)?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            { "name": "bundle", "resource": carried },
            { "name": "format", "valueCode": "flat" },
        ],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    let text = composition_text(&answer)?;
    assert!(
        text.contains('/'),
        "the format parameter selected the flat serialization: {text}"
    );
    Ok(())
}
