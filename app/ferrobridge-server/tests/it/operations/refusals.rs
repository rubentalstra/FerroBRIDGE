// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of a request the operations cannot serve.

use axum::body::Body;
use http::{Request, StatusCode, header};
use std::error::Error as StdError;

use super::FHIR_JSON;
use super::OPENEHR_JSON;
use super::PROFILE;
use super::app;
use super::call;
use super::composition;
use super::mapping_tree;

#[tokio::test]
async fn a_flat_composition_without_a_template_is_four_hundred_required()
-> Result<(), Box<dyn StdError>> {
    // "`templateId` ... is also required when submitting a flat Composition to
    // `$tofhir`" (rest-api.adoc, draft, section Query parameters).
    let root = mapping_tree()?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "composition",
            "valueString": r#"{"diagnose/problem_diagnosis/problem_diagnosis|value":"a"}"#,
        }],
    });
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    assert_eq!(Some("required"), answer["issue"][0]["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn another_media_type_is_four_hundred_fifteen() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    for route in [
        "/fhir/$tofhir",
        "/fhir/$toopenehr",
        "/fhir/tofhir",
        "/fhir/toopenehr",
    ] {
        let (status, media, body) = call(
            app(&root)?,
            Request::post(route)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("not a FHIR body"))?,
        )
        .await?;
        assert_eq!(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            status,
            "{route} read a text/plain body"
        );
        assert_eq!(FHIR_JSON, media, "{route}");
        let answer: serde_json::Value = serde_json::from_str(&body)?;
        assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    }
    Ok(())
}

#[tokio::test]
async fn an_openehr_body_on_the_enveloped_route_is_four_hundred_fifteen()
-> Result<(), Box<dyn StdError>> {
    // "The `Content-Type` and `Accept` headers on the operation endpoints ...
    // refer to the FHIR envelope, not to the openEHR payload it carries"
    // (rest-api.adoc, draft, section Media types).
    let root = mapping_tree()?;
    let (status, _, _) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, OPENEHR_JSON)
            .body(Body::from("{}"))?,
    )
    .await?;
    assert_eq!(StatusCode::UNSUPPORTED_MEDIA_TYPE, status);
    Ok(())
}

#[tokio::test]
async fn a_body_that_is_not_a_parameters_answers_an_operation_outcome()
-> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from("not json at all"))?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    assert_eq!(Some("error"), answer["issue"][0]["severity"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_mapping_that_cannot_run_answers_an_outcome_and_nothing_else()
-> Result<(), Box<dyn StdError>> {
    // Strictness is the default: a failed mapping answers an OperationOutcome
    // and no Bundle.
    let root = mapping_tree()?;
    let mut built = composition(app(&root)?).await?;
    if let Some(object) = built.as_object_mut() {
        object.remove("uid");
    }
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": built.to_string() }],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    assert!(
        answer["entry"].is_null() && answer["parameter"].is_null(),
        "a refusal carries no Bundle and no composition: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_bundle_with_two_subjects_is_refused_naming_them() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let mut first: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    let mut second = first.clone();
    for (resource, subject, id) in [
        (&mut first, "Patient/one", "one"),
        (&mut second, "Patient/two", "two"),
    ] {
        if let Some(object) = resource.as_object_mut() {
            object.insert(String::from("id"), serde_json::json!(id));
            object.insert(
                String::from("meta"),
                serde_json::json!({ "profile": [PROFILE] }),
            );
            object.insert(
                String::from("subject"),
                serde_json::json!({ "reference": subject }),
            );
        }
    }
    let carried = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{ "resource": first }, { "resource": second }],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(carried.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    assert!(
        body.contains("Patient/one") && body.contains("Patient/two"),
        "the outcome names the conflicting subjects: {body}"
    );
    Ok(())
}
