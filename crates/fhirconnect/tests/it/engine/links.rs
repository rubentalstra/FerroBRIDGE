// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `link` and `participationsFunction` families.

use core::error::Error;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::error::EngineError;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;

use openehr_mapping_core::index::paths::AqlPath;

use crate::support::compiled;
use crate::support::template;

use crate::engine::defaults;
use crate::engine::reference::condition_with;

/// A synthetic `ehr:` URI of a composition version the link targets.
const LINKED: &str = "ehr://ferrobridge.example/3a2f1c4e-0000-4000-8000-0000000000ab/compositions/d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1";

#[test]
fn a_link_is_written_and_read_back() -> Result<(), Box<dyn Error>> {
    // concept-mappings.adoc §Linked mappings: the fields of the LINK are the
    // ones the `link` block sets; RM 1.1.0 common.html §LINK.
    let program = compiled("ferrobridge_link")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({"encounter": {"reference": LINKED}}))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let entry = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]",
    ))?;
    let written = index
        .read(inbound.value(), entry, &[])?
        .ok_or("the entry was written")?;
    let link = written
        .pointer("/links/0")
        .ok_or("the entry carries a LINK")?;
    assert_eq!(
        link.pointer("/target/value")
            .and_then(serde_json::Value::as_str),
        Some(LINKED)
    );
    assert_eq!(
        link.pointer("/meaning/value")
            .and_then(serde_json::Value::as_str),
        Some("the encounter the diagnosis was made in")
    );
    assert_eq!(
        link.pointer("/type/value")
            .and_then(serde_json::Value::as_str),
        Some("encounter")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        outbound
            .value()
            .get("encounter")
            .and_then(|encounter| encounter.get("reference"))
            .and_then(Value::as_str),
        Some(LINKED),
        "the link's target reads back into the reference"
    );
    Ok(())
}

#[test]
fn a_link_to_what_no_ehr_uri_names_refuses() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 data_types.html §DV_EHR_URI: the target "has the scheme name
    // 'ehr'", and a relative FHIR reference has none.
    let program = compiled("ferrobridge_link")?;
    let index = template()?;
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "encounter": {"reference": "Encounter/synthetic-encounter-1"}
        }))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("a relative reference is no ehr: URI");
    assert!(
        matches!(error, EngineError::LinkTarget { ref target, .. } if target == "Encounter/synthetic-encounter-1"),
        "the refusal names the target: {error}"
    );
    Ok(())
}

#[test]
fn a_participation_on_the_event_context_is_written_and_read_back() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 ehr.html §EVENT_CONTEXT: `participations` is the same
    // List<PARTICIPATION> as ENTRY.other_participations, and Simplified
    // Formats writes it as `context/_participation:i` (master05 §EVENT_CONTEXT).
    let program = compiled("ferrobridge_context_participation")?;
    let index = template()?;
    let asserter = serde_json::json!({
        "reference": "Practitioner/synthetic-practitioner-2",
        "display": "Synthetic Practitioner Two"
    });
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({"asserter": asserter}))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let flat = index.flatten(inbound.value())?;
    assert!(
        flat.iter().any(
            |(key, value)| key.ends_with("context/_participation:0|function")
                && *value == serde_json::json!("asserter")
        ),
        "the context carries the participation: {flat:?}"
    );
    let context = index.node(&AqlPath::new("/context"))?;
    let written = index
        .read(inbound.value(), context, &[])?
        .ok_or("the context was written")?;
    assert_eq!(
        written
            .pointer("/participations/0/performer/name")
            .and_then(serde_json::Value::as_str),
        Some("Synthetic Practitioner Two")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        outbound
            .value()
            .get("asserter")
            .map(|found| found.to_serde_json(&mut fhir_types::codec::Path::root("Reference")))
            .transpose()?,
        Some(asserter),
    );
    Ok(())
}

#[test]
fn a_participation_is_written_and_read_back() -> Result<(), Box<dyn Error>> {
    // concept-mappings.adoc §Participation mappings: the function is the
    // method's, the participant the Reference; RM 1.1.0 common.html
    // §PARTICIPATION.
    let program = compiled("ferrobridge_participation")?;
    let index = template()?;
    let asserter = serde_json::json!({
        "reference": "Practitioner/synthetic-practitioner-1",
        "display": "Synthetic Practitioner One"
    });
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({"asserter": asserter}))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let entry = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]",
    ))?;
    let written = index
        .read(inbound.value(), entry, &[])?
        .ok_or("the entry was written")?;
    let participation = written
        .pointer("/other_participations/0")
        .ok_or("the entry carries a PARTICIPATION")?;
    assert_eq!(
        participation
            .pointer("/function/value")
            .and_then(serde_json::Value::as_str),
        Some("asserter")
    );
    assert_eq!(
        participation
            .pointer("/performer/name")
            .and_then(serde_json::Value::as_str),
        Some("Synthetic Practitioner One")
    );
    assert_eq!(
        participation
            .pointer("/performer/external_ref/namespace")
            .and_then(serde_json::Value::as_str),
        Some("Practitioner")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        outbound
            .value()
            .get("asserter")
            .map(|found| found.to_serde_json(&mut fhir_types::codec::Path::root("Reference")))
            .transpose()?,
        Some(asserter),
        "the participant reads back as the Reference it came from"
    );
    Ok(())
}
