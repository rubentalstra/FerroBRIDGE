// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `reference` mappings, resolved going in and created going out.

use core::cell::RefCell;
use core::error::Error;
use std::collections::BTreeMap;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::outcome::SkipReason;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::engine::seam::IdentityRequest;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::error::EngineError;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;

use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::paths::AqlPath;

use crate::support::compiled;
use crate::support::template;

use crate::engine::MapIdentities;
use crate::engine::MapReferences;
use crate::engine::defaults;

/// Returns the synthetic `Condition` with the given members set.
pub(super) fn condition_with(members: serde_json::Value) -> Result<Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let (Some(object), serde_json::Value::Object(added)) = (parsed.as_object_mut(), members) {
        object.extend(added);
    }
    Ok(Value::from_serde_json(parsed))
}

/// Returns the synthetic `Observation` the evidence reference points at.
fn evidence_observation() -> Value {
    Value::from_serde_json(evidence_json())
}

/// Returns the synthetic `Observation` the evidence reference points at, as
/// JSON.
fn evidence_json() -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Observation",
        "id": "synthetic-observation-1",
        "status": "final",
        "code": {"text": "Synthetic evidence one"}
    })
}

/// Returns the text of the problem-diagnosis comment a composition holds.
fn comment_of(
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
) -> Result<Option<String>, Box<dyn Error>> {
    let node = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]/items[at0069]/value",
    ))?;
    Ok(index
        .read(composition, node, &[])?
        .and_then(|value| value.get("value")?.as_str().map(String::from)))
}

#[test]
fn a_reference_maps_the_referenced_resource_into_openehr() -> Result<(), Box<dyn Error>> {
    // Reference.adoc: the `reference` mapping initializes the referenced
    // resource and its mappings run over it, with `$fhirRoot` its root.
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        evidence_observation(),
    );
    let references = MapReferences(held);
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(
        comment_of(&index, inbound.value())?.as_deref(),
        Some("Synthetic evidence one"),
        "the referenced Observation's code reached the composition"
    );
    Ok(())
}

#[test]
fn a_contained_reference_resolves_in_the_document_itself() -> Result<(), Box<dyn Error>> {
    // R4 references.html#contained: `#id` names a contained resource.
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut contained = evidence_json();
    if let Some(object) = contained.as_object_mut() {
        object.insert(String::from("id"), serde_json::json!("evidence"));
    }
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "contained": [contained],
            "evidence": [{"detail": [{"reference": "#evidence"}]}]
        }))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(
        comment_of(&index, inbound.value())?.as_deref(),
        Some("Synthetic evidence one")
    );
    Ok(())
}

#[test]
fn a_reference_that_resolves_to_nothing_is_a_declared_skip() -> Result<(), Box<dyn Error>> {
    // references.adoc: "If not possible, the engine proceeds with the mapping".
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/absent"}]}]
        }))?,
        &Seams::default().with_references(&MapReferences::default()),
        &defaults(),
        &CallContext::new(),
    )?;
    assert!(
        inbound.warnings().contains(&Warning::Skipped {
            mapping: String::from("evidence"),
            reason: SkipReason::UnresolvedReference {
                reference: String::from("Observation/absent"),
            },
        }),
        "the unresolved reference is declared: {:?}",
        inbound.warnings()
    );
    assert_eq!(comment_of(&index, inbound.value())?, None);
    Ok(())
}

#[test]
fn a_reference_of_the_wrong_type_refuses() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        Value::from_serde_json(serde_json::json!({"resourceType": "Specimen", "id": "x"})),
    );
    let references = MapReferences(held);
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("a Specimen is no Observation");
    assert!(
        matches!(error, EngineError::Reference { .. }),
        "the refusal is the reference's: {error}"
    );
    Ok(())
}

#[test]
fn a_reference_out_of_openehr_creates_the_resource_the_sink_names() -> Result<(), Box<dyn Error>> {
    // Reference.adoc: "initialize a new resource in FHIR ... and reference it
    // inside the resource we are currently mapping".
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        evidence_observation(),
    );
    let references = MapReferences(held);
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )?;
    let mut ids = BTreeMap::new();
    ids.insert(
        String::from("Observation"),
        String::from("created-observation-1"),
    );
    let identities = MapIdentities {
        ids,
        seen: RefCell::default(),
    };
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default().with_identities(&identities),
        &CallContext::new(),
    )?;
    let detail = outbound
        .value()
        .get("evidence")
        .and_then(|evidence| evidence.as_array()?.first())
        .and_then(|evidence| evidence.get("detail"))
        .and_then(|detail| detail.as_array()?.first())
        .and_then(|detail| detail.get("reference"))
        .and_then(Value::as_str);
    assert_eq!(detail, Some("Observation/created-observation-1"));
    let created = outbound
        .created()
        .first()
        .ok_or("the run created the Observation")?;
    assert_eq!(
        created.get("id").and_then(Value::as_str),
        Some("created-observation-1")
    );
    assert_eq!(
        created
            .get("code")
            .and_then(|code| code.get("text"))
            .and_then(Value::as_str),
        Some("Synthetic evidence one"),
        "the created Observation carries what its mappings read"
    );
    let seen = identities.seen.borrow();
    assert_eq!(
        seen.iter()
            .map(IdentityRequest::resource_type)
            .collect::<Vec<&str>>(),
        ["Observation"],
        "one identity was asked for"
    );
    Ok(())
}

#[test]
fn a_reference_chain_that_reaches_itself_refuses() -> Result<(), Box<dyn Error>> {
    // references.adoc: the engine should "keep track of which ones are
    // already resolved" to prevent circular dependencies.
    let program = compiled("ferrobridge_reference_cycle")?;
    let index = template()?;
    let mut observation = evidence_json();
    if let Some(object) = observation.as_object_mut() {
        object.insert(
            String::from("hasMember"),
            serde_json::json!([{"reference": "Observation/synthetic-observation-1"}]),
        );
    }
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        Value::from_serde_json(observation),
    );
    let references = MapReferences(held);
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("the chain reaches the Observation twice");
    assert!(
        matches!(error, EngineError::ReferenceCycle { ref reference, .. } if reference == "Observation/synthetic-observation-1"),
        "the refusal names the reference: {error}"
    );
    Ok(())
}
