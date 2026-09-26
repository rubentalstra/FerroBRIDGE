// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `hierarchy.split` groups.

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
use fhirconnect::engine::traverse::error::SplitRefusal;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;

use fhirconnect::resolve::program::hierarchy::Preprocessor;
use fhirconnect::resolve::program::mapping::Mapping;
use fhirconnect::resolve::program::mapping::MappingParts;
use fhirconnect::resolve::program::mapping::Method;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::header::metadata::MappingName;

use crate::support::compiled;
use crate::support::template;

use crate::engine::MapIdentities;
use crate::engine::chain::cyclic_program;
use crate::engine::condition_document;
use crate::engine::defaults;
use crate::engine::mapping;
use crate::engine::reference::condition_with;
use crate::engine::slot_parts;

/// Returns the `bodySite.text` values of one Condition, in order.
fn body_sites(condition: &Value) -> Vec<&str> {
    condition
        .get("bodySite")
        .and_then(Value::as_array)
        .map(|sites| {
            sites
                .iter()
                .filter_map(|site| site.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// Returns the body-site names the anatomical-location clusters of a
/// composition carry, in order.
fn cluster_sites(composition: &CanonicalComposition) -> Vec<String> {
    composition
        .value()
        .pointer("/content/0/data/items")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("archetype_node_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("openEHR-EHR-CLUSTER.anatomical_location.v1")
                })
                .filter_map(|cluster| {
                    cluster
                        .pointer("/items/0/value/value")
                        .and_then(serde_json::Value::as_str)
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns a Condition with one `bodySite` per text.
fn condition_with_sites(texts: &[&str]) -> Result<Value, Box<dyn Error>> {
    let sites: Vec<serde_json::Value> = texts
        .iter()
        .map(|text| serde_json::json!({"text": text}))
        .collect();
    condition_with(serde_json::json!({"bodySite": sites}))
}

#[test]
fn a_split_into_openehr_creates_one_cluster_per_body_site() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §Hierarchy and unique values: from FHIR to
    // openEHR, each occurrence of the `with` path creates a new element.
    let program = compiled("ferrobridge_split")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(cluster_sites(inbound.value()), ["Left knee", "Right knee"]);
    let subject_skips = inbound
        .warnings()
        .iter()
        .filter(|warning| {
            **warning
                == Warning::Skipped {
                    mapping: String::from("subject"),
                    reason: SkipReason::Unidirectional,
                }
        })
        .count();
    assert_eq!(
        subject_skips,
        1,
        "a skip that holds for every group is declared once: {:?}",
        inbound.warnings()
    );
    Ok(())
}

#[test]
fn a_split_out_of_openehr_creates_one_resource_per_cluster() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §split: "for each event in openEHR, one
    // resource must be created", and the composition's other fields reach
    // every one of them.
    let program = compiled("ferrobridge_split")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let identities = MapIdentities {
        ids: BTreeMap::from([(String::from("Condition"), String::from("split-condition-2"))]),
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
    assert_eq!(body_sites(outbound.value()), ["Left knee"]);
    let [ref second] = *outbound.created() else {
        return Err(Box::from("the second cluster created one more Condition"));
    };
    assert_eq!(body_sites(second), ["Right knee"]);
    assert_eq!(
        second.get("id").and_then(Value::as_str),
        Some("split-condition-2")
    );
    for condition in [outbound.value(), second] {
        assert_eq!(
            condition
                .get("code")
                .and_then(|code| code.get("text"))
                .and_then(Value::as_str),
            Some("Synthetic problem one"),
            "the content outside the split reaches every resource"
        );
    }
    let seen = identities.seen.borrow();
    let [ref request] = *seen.as_slice() else {
        return Err(Box::from("one identity was asked for"));
    };
    assert_eq!(
        request.occurrence(),
        [2],
        "the split occurrence travels in the identity request"
    );
    Ok(())
}

#[test]
fn a_split_derives_the_same_ids_on_every_run() -> Result<(), Box<dyn Error>> {
    // The default identity sink derives the id from the request, so the same
    // composition yields the same ids across runs.
    let program = compiled("ferrobridge_split")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee", "Left hip"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let ids = || -> Result<Vec<String>, Box<dyn Error>> {
        let outbound = to_fhir(
            &program,
            &SCHEMAS,
            &index,
            inbound.value(),
            &Seams::default(),
            &CallContext::new(),
        )?;
        Ok(outbound
            .created()
            .iter()
            .filter_map(|created| created.get("id").and_then(Value::as_str).map(String::from))
            .collect())
    };
    let first = ids()?;
    assert_eq!(
        first.len(),
        2,
        "two further clusters created two Conditions"
    );
    assert_ne!(
        first.first(),
        first.get(1),
        "each occurrence takes its own id"
    );
    assert_eq!(first, ids()?, "a second run derives the same ids");
    Ok(())
}

#[test]
fn a_split_groups_occurrences_by_their_unique_tuple() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc: "the `unique:` key defines the condition that
    // triggers the creation of the new element", so occurrences that share
    // the tuple share the element.
    let program = compiled("ferrobridge_split_unique")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee", "Left knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(
        cluster_sites(inbound.value()),
        ["Left knee", "Right knee"],
        "two distinct body-site texts create two clusters"
    );
    let identities = MapIdentities {
        ids: BTreeMap::from([(String::from("Condition"), String::from("unique-condition"))]),
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
    assert_eq!(outbound.created().len(), 1);
    let seen = identities.seen.borrow();
    assert_eq!(
        seen.first().map(IdentityRequest::unique),
        Some([String::from("Right knee")].as_slice()),
        "the unique tuple travels in the identity request"
    );
    Ok(())
}

#[test]
fn a_slotted_files_split_runs_under_the_slot() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc: the hierarchy lives in the preprocessor of the
    // file it belongs to, so a slotted file's split runs over the slotted
    // mappings.
    let split = compiled("ferrobridge_split")?;
    let index = template()?;
    let model = MappingName::new("ferrobridge_split")?;
    let slot = Mapping::new(MappingParts {
        method: Method::Slot {
            model: model.clone(),
            preprocessors: vec![Preprocessor::new(
                model.clone(),
                None,
                None,
                split.hierarchy().cloned(),
            )],
            mappings: split.mappings().to_vec(),
        },
        ..slot_parts(&model, Vec::new())
    });
    let host = cyclic_program(&MappingName::new("ferrobridge_slot_host")?, vec![slot])?;
    let inbound = to_openehr(
        &host,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(cluster_sites(inbound.value()), ["Left knee", "Right knee"]);
    let outbound = to_fhir(
        &host,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(body_sites(outbound.value()), ["Left knee"]);
    let [ref second] = *outbound.created() else {
        return Err(Box::from("the slotted split created one more Condition"));
    };
    assert_eq!(body_sites(second), ["Right knee"]);
    Ok(())
}

#[test]
fn a_split_on_a_node_that_does_not_repeat_refuses() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §split: an element is created per occurrence, so
    // a node the template holds once takes no second one.
    let split = compiled("ferrobridge_split")?;
    let minimal = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let problem = mapping(&minimal, "problemDiagnose").ok_or("the problem mapping")?;
    let fhir = problem.fhir().cloned();
    let openehr = problem.openehr().cloned();
    let hierarchy = fhirconnect::resolve::program::hierarchy::Hierarchy::new(
        fhir,
        openehr,
        None,
        split
            .hierarchy()
            .and_then(|found| found.split_openehr())
            .cloned(),
    );
    let model = MappingName::new("ferrobridge_split")?;
    let slot = Mapping::new(MappingParts {
        method: Method::Slot {
            model: model.clone(),
            preprocessors: vec![Preprocessor::new(
                model.clone(),
                None,
                None,
                Some(hierarchy),
            )],
            mappings: split.mappings().to_vec(),
        },
        ..slot_parts(&model, Vec::new())
    });
    let host = cyclic_program(&MappingName::new("ferrobridge_slot_host")?, vec![slot])?;
    let error = to_openehr(
        &host,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("the problem name occurs once");
    assert!(
        matches!(
            error,
            EngineError::Split {
                reason: SplitRefusal::NotRepeating { .. },
                ..
            }
        ),
        "the refusal names the node: {error}"
    );
    Ok(())
}
