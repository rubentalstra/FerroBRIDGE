// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two operations chained, a condition to a composition and back.

use axum::body::Body;
use http::{Request, StatusCode, header};
use std::error::Error as StdError;

use super::FHIR_JSON;
use super::PROFILE;
use super::app;
use super::bundle;
use super::call;
use super::composition_text;
use super::mapping_tree;

/// Returns the one `Condition` a `$tofhir` Bundle carries.
fn mapped_condition(answer: &serde_json::Value) -> Result<&serde_json::Value, Box<dyn StdError>> {
    answer["entry"]
        .as_array()
        .ok_or("the Bundle carries entries")?
        .iter()
        .map(|entry| &entry["resource"])
        .find(|resource| resource["resourceType"].as_str() == Some("Condition"))
        .ok_or_else(|| Box::<dyn StdError>::from("the Bundle carries the mapped Condition"))
}

/// Runs `$toopenehr` then `$tofhir` over `condition` and returns what the
/// second leg mapped back.
///
/// `patient` is the `context.patient` reference the second leg carries, for
/// a program that maps no subject of its own.
async fn both_legs(
    root: &tempfile::TempDir,
    condition: &serde_json::Value,
    patient: Option<serde_json::Value>,
) -> Result<serde_json::Value, Box<dyn StdError>> {
    let carried = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{ "resource": condition }],
    });
    let (status, _, body) = call(
        app(root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(carried.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    let mut built: serde_json::Value = serde_json::from_str(composition_text(&answer)?)?;
    // NOTE: no specification governs this: our own design, a composition
    // takes its `uid` when a CDR commits it, so the round trip without one
    // gives it the version a commit would, as `composition` above does.
    if let Some(object) = built.as_object_mut() {
        object.insert(
            String::from("uid"),
            serde_json::json!({
                "_type": "OBJECT_VERSION_ID",
                "value": "d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1",
            }),
        );
    }
    let mut parameters = vec![serde_json::json!({
        "name": "composition",
        "valueString": built.to_string(),
    })];
    if let Some(reference) = patient {
        parameters.push(serde_json::json!({
            "name": "context",
            "part": [{ "name": "patient", "valueReference": reference }],
        }));
    }
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": parameters,
    });
    let (status, _, body) = call(
        app(root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    Ok(mapped_condition(&answer)?.clone())
}

#[tokio::test]
async fn the_two_operations_round_trip_a_condition_modulo_what_the_program_maps()
-> Result<(), Box<dyn StdError>> {
    // The synthetic program maps the diagnosis name both ways and writes the
    // subject going to FHIR only, so every other element of the input is the
    // declared loss of this round trip, the subject comes back as the literal
    // reference the program writes, and the id is the one `$tofhir` derives
    // from the composition's version.
    let root = mapping_tree()?;
    let input =
        serde_json::from_str::<serde_json::Value>(&bundle()?)?["entry"][0]["resource"].clone();
    let output = both_legs(&root, &input, None).await?;
    let set = ferrobridge_testkit::laws::declared(&[], &[], &input, &output);
    assert_eq!(
        set["lost"],
        serde_json::json!([
            "clinicalStatus.coding[0].code = \"active\"",
            "clinicalStatus.coding[0].display = \"Active\"",
            "clinicalStatus.coding[0].system = \"http://terminology.hl7.org/CodeSystem/condition-clinical\"",
            "identifier[0].system = \"http://example.org/fhir/sid/ferrobridge-condition\"",
            "identifier[0].value = \"synthetic-condition-0001\"",
            format!("meta.profile[0] = \"{PROFILE}\""),
            "recordedDate = \"2026-09-12T10:00:00+02:00\"",
            "subject.identifier.system = \"http://example.org/fhir/sid/ferrobridge-subject\"",
            "subject.identifier.value = \"synthetic-subject-0001\"",
            "verificationStatus.coding[0].code = \"confirmed\"",
            "verificationStatus.coding[0].display = \"Confirmed\"",
            "verificationStatus.coding[0].system = \"http://terminology.hl7.org/CodeSystem/condition-ver-status\"",
        ]),
        "{set}"
    );
    assert_eq!(
        set["added"],
        serde_json::json!(["subject.reference = \"Patient/synthetic-subject-0001\""]),
        "{set}"
    );
    assert_eq!(
        set["changed"],
        serde_json::json!([
            "id: \"ferrobridge-synthetic-condition-1\" became \"d2b3c1a0-0000-4000-8000-000000000001\""
        ]),
        "{set}"
    );
    Ok(())
}

/// The KDS program maps no subject, and `Condition.subject` is `1..1`
/// (<https://hl7.org/fhir/R4/condition.html>), so the second leg carries the
/// input's subject as `context.patient`, which "takes precedence"
/// (`engine/rest-api.adoc` §Resolving the patient). The wire's declared set
/// is then the engine's with exactly two rows moved: the subject is not lost,
/// and the id is the one `$tofhir` derives from the composition's version.
#[tokio::test]
async fn the_two_operations_round_trip_the_kds_condition_as_the_engine_does()
-> Result<(), Box<dyn StdError>> {
    let root = tempfile::tempdir()?;
    crate::kds::write_mappings(&root.path().join("mappings"))?;
    std::fs::create_dir_all(root.path().join("templates"))?;
    std::fs::write(
        root.path().join("templates").join("KDS_Diagnose.opt"),
        ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT,
    )?;
    let input: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_CONDITION)?;
    let patient = serde_json::json!({ "identifier": input["subject"]["identifier"].clone() });
    let output = both_legs(&root, &input, Some(patient)).await?;
    let set = ferrobridge_testkit::laws::declared(&[], &[], &input, &output);
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/fhirconnect/tests/it/snapshots/it__roundtrip__kds_putget.snap"
    ))?;
    let body = pinned
        .splitn(3, "---")
        .nth(2)
        .ok_or("the snapshot carries a body")?;
    let engine: serde_json::Value = serde_json::from_str(body)?;
    let id = format!("id = {}", input["id"]);
    let moved = |row: &&serde_json::Value| {
        row.as_str()
            .is_some_and(|row| row == id || row.starts_with("subject.identifier."))
    };
    let engine_lost: Vec<&serde_json::Value> = engine["lost"]
        .as_array()
        .ok_or("the engine's lost list")?
        .iter()
        .filter(|row| !moved(row))
        .collect();
    assert_eq!(
        engine["lost"].as_array().map(Vec::len),
        Some(engine_lost.len().saturating_add(3)),
        "the engine lost the id and the two subject identifier rows: {engine}"
    );
    assert_eq!(set["lost"], serde_json::json!(engine_lost), "{set}");
    assert_eq!(set["added"], engine["added"], "{set}");
    assert_eq!(engine["changed"], serde_json::json!([]), "{engine}");
    assert_eq!(
        set["changed"],
        serde_json::json!([format!(
            "id: {} became \"d2b3c1a0-0000-4000-8000-000000000001\"",
            input["id"]
        )]),
        "{set}"
    );
    Ok(())
}
