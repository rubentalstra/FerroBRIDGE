// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data-type pair a mapping with no `type` key converts through.
//!
//! The `type` key is deprecated because the type "is derivable from the
//! instances of FHIR and openEHR"
//! (`types-of-mappings/data-type/data-mappings.adoc`, §Deprecated), so the
//! compiler derives the pair from the element table and the Web Template, and
//! the engine runs what it derived. The required-child rule of
//! `engine/Fail.adoc` is asserted here too, over a structural node, with the
//! other rules the published KDS chain needed: one instance of the start
//! archetype per resource, and a manual entry's `openehrCondition` going out
//! of openEHR.

use core::error::Error;
use std::sync::Arc;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::Defaults;
use fhirconnect::engine::traverse::EngineError;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Derived;
use fhirconnect::resolve::program::Mapping;
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;

use crate::resolve::codes;
use crate::resolve::context;
use crate::resolve::model;
use crate::resolve::program_of;
use crate::resolve::set_of;
use crate::resolve::start_context;
use crate::resolve::start_model;
use crate::support::compiled;
use crate::support::template;

/// The instant the runs of this module default their composition fields to.
const NOW: &str = "2026-09-13T08:00:00Z";

/// The extension url the fixture's `certainty` mapping matches on.
const CERTAINTY: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-certainty";

/// Returns the top-level mapping of `program` named `name`.
fn top<'program>(program: &'program Program, name: &str) -> Result<&'program Mapping, String> {
    program
        .mappings()
        .iter()
        .find(|mapping| mapping.name() == name)
        .ok_or_else(|| format!("no mapping named {name}"))
}

/// Returns a synthetic `Condition` carrying the problem name and `extra`.
fn condition(extra: &serde_json::Value) -> Value {
    let mut resource = serde_json::json!({
        "resourceType": "Condition",
        "code": {
            "coding": [{
                "system": "http://example.org/fhir/CodeSystem/synthetic-problems",
                "code": "SYN-001",
                "display": "Synthetic problem one"
            }],
            "text": "Synthetic problem one"
        }
    });
    if let (Some(object), Some(more)) = (resource.as_object_mut(), extra.as_object()) {
        for (key, value) in more {
            object.insert(key.clone(), value.clone());
        }
    }
    Value::from_serde_json(resource)
}

/// Maps `resource` into openEHR through the derived-pair fixture.
fn inbound(
    program: &Program,
    index: &WebTemplateIndex,
    resource: &Value,
    defaults: &Defaults,
) -> Result<(CanonicalComposition, Vec<Warning>), EngineError> {
    let outcome = to_openehr(
        program,
        &SCHEMAS,
        index,
        resource,
        &Seams::default(),
        defaults,
        &CallContext::new(),
    )?;
    let warnings = outcome.warnings().to_vec();
    Ok((outcome.into_value(), warnings))
}

/// Maps `composition` back into FHIR as plain JSON.
fn outbound(
    program: &Program,
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let outcome = to_fhir(
        program,
        &SCHEMAS,
        index,
        composition,
        &Seams::default(),
        &CallContext::new(),
    )?;
    Ok(outcome
        .value()
        .to_serde_json(&mut fhir_types::codec::Path::root("Condition"))?)
}

/// Returns the FLAT value whose key ends with `suffix`.
fn flat_value(
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    suffix: &str,
) -> Result<Option<serde_json::Value>, Box<dyn Error>> {
    Ok(index
        .flatten(composition)?
        .into_iter()
        .find(|(key, _)| key.ends_with(suffix))
        .map(|(_, value)| value))
}

/// Returns the defaults every run of this module takes.
fn defaults() -> Defaults {
    Defaults::at(NOW).with_language("en").with_territory("NL")
}

#[test]
fn an_untyped_date_time_element_converts_through_the_date_time_pair() -> Result<(), Box<dyn Error>>
{
    // DV_DATE_TIME.adoc §DateTime pairs dateTime with DV_DATE_TIME.
    let program = compiled("ferrobridge_derived")?;
    assert_eq!(
        top(&program, "recorded")?.derived(),
        Some(&Derived::Element("dateTime"))
    );
    let index = template()?;
    let recorded = "2026-09-12T10:00:00.123+02:00";
    let (composition, warnings) = inbound(
        &program,
        &index,
        &condition(&serde_json::json!({ "recordedDate": recorded })),
        &defaults(),
    )?;
    assert!(
        !warnings.iter().any(|warning| matches!(
            *warning,
            Warning::Defaulted { ref field } if field == "/context/start_time"
        )),
        "the mapping wrote the start time: {warnings:?}"
    );
    assert_eq!(
        flat_value(&index, &composition, "ctx/time")?,
        Some(serde_json::json!(recorded))
    );
    let back = outbound(&program, &index, &composition)?;
    assert_eq!(back["recordedDate"], serde_json::json!(recorded));
    Ok(())
}

#[test]
fn an_untyped_structural_node_with_children_anchors_them() -> Result<(), Box<dyn Error>> {
    // FollowedBy.adoc writes `type: NONE` on a mapping that only anchors its
    // children; with no `type` the node's class says the same.
    let program = compiled("ferrobridge_derived")?;
    assert_eq!(top(&program, "entry")?.derived(), Some(&Derived::Anchor));
    let index = template()?;
    let (composition, _) = inbound(
        &program,
        &index,
        &condition(&serde_json::json!({})),
        &defaults(),
    )?;
    let back = outbound(&program, &index, &composition)?;
    assert_eq!(
        back["code"]["coding"][0]["code"],
        serde_json::json!("SYN-001"),
        "the child under the anchor ran both ways: {back}"
    );
    Ok(())
}

#[test]
fn a_choice_with_no_type_filter_reads_the_alternative_the_document_carries()
-> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_derived")?;
    let index = template()?;
    for (carried, value) in [
        (
            serde_json::json!({ "onsetDateTime": "2026-09-01T08:00:00+02:00" }),
            "2026-09-01T08:00:00+02:00",
        ),
        // DV_DATE_TIME.adoc §Period: the start fills the value.
        (
            serde_json::json!({ "onsetPeriod": { "start": "2026-09-02T08:00:00+02:00" } }),
            "2026-09-02T08:00:00+02:00",
        ),
    ] {
        let (composition, _) = inbound(&program, &index, &condition(&carried), &defaults())?;
        let found = index
            .flatten(&composition)?
            .into_iter()
            .find(|(_, held)| *held == serde_json::json!(value));
        assert!(found.is_some(), "{carried} reached DV_DATE_TIME");
    }
    Ok(())
}

#[test]
fn a_choice_with_no_type_filter_is_written_as_the_first_pair_of_the_node_class()
-> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_derived")?;
    let Some(&Derived::Choice {
        write: Some(ref written),
        ..
    }) = top(&program, "onset")?.derived()
    else {
        return Err("the onset mapping derived a written alternative".into());
    };
    assert_eq!(written.code(), "dateTime");
    let index = template()?;
    let (composition, _) = inbound(
        &program,
        &index,
        &condition(&serde_json::json!({ "onsetPeriod": { "start": "2026-09-02T08:00:00+02:00" } })),
        &defaults(),
    )?;
    let back = outbound(&program, &index, &composition)?;
    assert_eq!(
        back["onsetDateTime"],
        serde_json::json!("2026-09-02T08:00:00+02:00"),
        "{back}"
    );
    assert!(back.get("onsetPeriod").is_none(), "{back}");
    Ok(())
}

#[test]
fn an_extension_against_a_data_value_converts_through_its_value() -> Result<(), Box<dyn Error>> {
    // An extension carries its data in `value[x]`
    // (<https://hl7.org/fhir/R4/extensibility.html>).
    let program = compiled("ferrobridge_derived")?;
    let index = template()?;
    let carried = serde_json::json!({
        "extension": [{
            "url": CERTAINTY,
            "valueCoding": {
                "system": "http://example.org/fhir/CodeSystem/ferrobridge-certainty",
                "code": "SYN-C",
                "display": "Synthetic certainty"
            }
        }]
    });
    let (composition, _) = inbound(&program, &index, &condition(&carried), &defaults())?;
    let back = outbound(&program, &index, &composition)?;
    let extension = &back["extension"][0];
    assert_eq!(extension["url"], serde_json::json!(CERTAINTY), "{back}");
    assert_eq!(
        extension["valueCodeableConcept"]["coding"][0]["code"],
        serde_json::json!("SYN-C"),
        "DV_CODED_TEXT is written as its first pair, CodeableConcept: {back}"
    );
    Ok(())
}

#[test]
fn the_entry_provider_travels_as_its_flat_family() -> Result<(), Box<dyn Error>> {
    // ENTRY.provider is a PARTY_PROXY, and PARTY_IDENTIFIED.adoc pairs it with
    // a Reference.
    let program = compiled("ferrobridge_derived")?;
    let index = template()?;
    let (composition, _) = inbound(
        &program,
        &index,
        &condition(&serde_json::json!({ "recorder": { "display": "Synthetic recorder" } })),
        &defaults(),
    )?;
    assert_eq!(
        flat_value(&index, &composition, "/_provider|name")?,
        Some(serde_json::json!("Synthetic recorder"))
    );
    let back = outbound(&program, &index, &composition)?;
    assert_eq!(
        back["recorder"]["display"],
        serde_json::json!("Synthetic recorder")
    );
    Ok(())
}

#[test]
fn the_event_context_setting_is_a_declared_default() -> Result<(), Box<dyn Error>> {
    // engine/defaults-for-fields.adoc: the setting "can be defaulted to one of
    // the valid values".
    let program = compiled("ferrobridge_derived")?;
    let index = template()?;
    let resource = condition(&serde_json::json!({}));
    let (composition, warnings) = inbound(&program, &index, &resource, &defaults())?;
    assert!(
        warnings.iter().any(|warning| matches!(
            *warning,
            Warning::Defaulted { ref field } if field == "/context/setting"
        )),
        "{warnings:?}"
    );
    assert_eq!(
        flat_value(&index, &composition, "ctx/setting")?,
        Some(serde_json::json!("238"))
    );
    let (chosen, _) = inbound(
        &program,
        &index,
        &resource,
        // NOTE: openehr_terminology.xml, the `setting` group names 229 "primary
        // nursing care" (228 is "primary medical care"), so the pair is one concept.
        &defaults().with_setting("229", "primary nursing care"),
    )?;
    assert_eq!(
        flat_value(&index, &chosen, "ctx/setting")?,
        Some(serde_json::json!("229"))
    );
    Ok(())
}

#[test]
fn a_setting_whose_code_and_value_are_two_concepts_is_refused() -> Result<(), Box<dyn Error>> {
    // engine/defaults-for-fields.adoc links the openEHR `setting` group, in
    // which 229 is "primary nursing care"; the FLAT builder takes the rubric
    // of the code (Simplified Formats master06 §setting), so a pair naming two
    // concepts would store a value the project did not give.
    let program = compiled("ferrobridge_derived")?;
    let index = template()?;
    let resource = condition(&serde_json::json!({}));
    let error = inbound(
        &program,
        &index,
        &resource,
        &defaults().with_setting("229", "primary medical care"),
    )
    .err()
    .ok_or("a setting of two concepts was accepted")?;
    assert!(
        error.to_string().contains("/context/setting"),
        "the refusal names the node: {error}"
    );
    Ok(())
}

/// Compiles one small synthetic case against the diagnosis template.
fn synthetic(body: &str) -> Result<Arc<Program>, Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(body)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(name, text)| (*name, text.as_str()))
        .collect();
    let set = set_of(&borrowed)?;
    program_of(&set, "synthetic.context")
        .map_err(|diagnostics| codes(&diagnostics).join(", ").into())
}

/// Returns the codes a small synthetic case is refused with.
fn refused(body: &str) -> Result<Vec<String>, Box<dyn Error>> {
    match synthetic(body) {
        Ok(program) => Err(format!("the case compiled: {program}").into()),
        Err(error) => Ok(error.to_string().split(", ").map(String::from).collect()),
    }
}

#[test]
fn a_choice_the_node_class_pairs_with_no_alternative_of_is_refused() -> Result<(), Box<dyn Error>> {
    // Condition.onset[x] admits dateTime, Age, Period, Range and string, and
    // the DV_CODED_TEXT of items[at0073] pairs with CodeableConcept and Coding.
    let found = refused(
        "mappings:\n  - name: \"onsetName\"\n    with:\n      fhir: \"$resource.onset\"\n      openehr: \"$archetype/data[at0001]/items[at0073]\"\n",
    )?;
    assert_eq!(found, vec!["fc-underived-alternative"]);
    Ok(())
}

#[test]
fn a_choice_only_read_needs_no_written_alternative() -> Result<(), Box<dyn Error>> {
    let program = synthetic(
        "mappings:\n  - name: \"onsetName\"\n    unidirectional: \"fhir->openEHR\"\n    with:\n      fhir: \"$resource.onset\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n",
    )?;
    assert!(matches!(
        top(&program, "onsetName")?.derived(),
        Some(&Derived::Choice { write: None, .. })
    ));
    Ok(())
}

#[test]
fn a_type_key_on_a_choice_fixes_the_alternative() -> Result<(), Box<dyn Error>> {
    let program = synthetic(
        "mappings:\n  - name: \"onset\"\n    with:\n      fhir: \"$resource.onset\"\n      openehr: \"$archetype/data[at0001]/items[at0077]\"\n      type: \"DATETIME\"\n",
    )?;
    let Some(Derived::Declared(declared)) = top(&program, "onset")?.derived() else {
        return Err("the typed choice derived its alternative".into());
    };
    assert_eq!(declared.code(), "dateTime");
    assert_eq!(
        declared.target().expression().as_str(),
        "$resource.onset.as(DateTime)"
    );
    Ok(())
}

#[test]
fn a_type_key_the_choice_does_not_admit_is_refused() -> Result<(), Box<dyn Error>> {
    let found = refused(
        "mappings:\n  - name: \"onset\"\n    with:\n      fhir: \"$resource.onset\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n      type: \"CODING\"\n",
    )?;
    assert_eq!(found, vec!["fc-underived-alternative"]);
    Ok(())
}

#[test]
fn an_untyped_structural_node_with_no_child_compiles_with_a_warning() -> Result<(), Box<dyn Error>>
{
    let program = synthetic(
        "mappings:\n  - name: \"entry\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype\"\n",
    )?;
    assert_eq!(top(&program, "entry")?.derived(), None);
    let warned: Vec<String> = program
        .warnings()
        .iter()
        .map(|warning| warning.code().to_string())
        .collect();
    assert_eq!(warned, vec!["fc-anchor-without-children"]);
    Ok(())
}

/// A model mapping of the published `KDS_Diagnose` template whose
/// `EVALUATION`, `1..1` below the composition, is a required structural
/// child of the composition root, with a top-level mapping after it that
/// writes the problem name the template requires of the entry.
fn kds_required(child_condition: &str) -> String {
    model(
        "EVALUATION.kds.v1",
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
        &format!(
            "mappings:\n  - name: \"root\"\n    with:\n      fhir: \"$resource\"\n      openehr: \"$composition\"\n      type: \"NONE\"\n    followedBy:\n      mappings:\n        - name: \"entry\"\n          with:\n            fhir: \"$resource.code\"\n            openehr: \"$archetype\"\n{child_condition}          followedBy:\n            mappings:\n              - name: \"name\"\n                with:\n                  fhir: \"$fhirRoot\"\n                  openehr: \"$archetype/data[at0001]/items[at0002]\"\n                  type: \"CODEABLECONCEPT\"\n  - name: \"problem\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n      type: \"CODEABLECONCEPT\"\n"
        ),
    )
}

/// Compiles a `KDS_Diagnose` case and maps `resource` into openEHR.
fn kds_run(
    model_file: &str,
    resource: &Value,
) -> Result<Result<CanonicalComposition, EngineError>, Box<dyn Error>> {
    let context_file = context(
        "kds.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/kds\"\n  template:\n    id: \"KDS_Diagnose\"\n  archetypes:\n    - \"EVALUATION.kds.v1\"\n  start: \"EVALUATION.kds.v1\"\n",
    );
    let set = set_of(&[("model.yml", model_file), ("context.yml", &context_file)])?;
    let index = crate::roundtrip::kds_template()?;
    let program = compile(
        &set,
        &MappingName::new("kds.context")?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .map_err(|diagnostics| codes(&diagnostics).join(", "))?;
    Ok(to_openehr(
        &program,
        &SCHEMAS,
        &index,
        resource,
        &Seams::default(),
        &Defaults::at(NOW).with_language("de").with_territory("DE"),
        &CallContext::new(),
    )
    .map(fhirconnect::engine::outcome::Outcome::into_value))
}

#[test]
fn one_resource_maps_into_one_instance_of_the_start_archetype() -> Result<(), Box<dyn Error>> {
    // basics/Variables.adoc: `$archetype` is the root of the archetype as
    // `$resource` is the root of the resource, and KDS_Diagnose lets the
    // entry repeat, so two top-level mappings write into the one instance.
    let composition = kds_run(&kds_required(""), &condition(&serde_json::json!({})))??;
    let index = crate::roundtrip::kds_template()?;
    let keys: Vec<String> = index.flatten(&composition)?.keys().cloned().collect();
    assert!(
        keys.iter()
            .any(|key| key.starts_with("diagnose/diagnose:0/")),
        "{keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|key| key.starts_with("diagnose/diagnose:1")),
        "a second mapping appended no second entry: {keys:?}"
    );
    Ok(())
}

#[test]
fn a_manual_entry_writes_fhir_only_where_its_openehr_condition_holds() -> Result<(), Box<dyn Error>>
{
    // basics/Conditions.adoc: "conditions are always applied on the input
    // data", and going out of openEHR the input is the composition.
    let entry = |name: &str, code: &str, local: &str| {
        format!(
            "      - name: \"{name}\"\n        openehr:\n          - path: \"defining_code/terminology_id/value\"\n            value: \"local\"\n          - path: \"defining_code/code_string\"\n            value: \"{local}\"\n          - path: \"value\"\n            value: \"{name}\"\n        fhirCondition:\n          targetRoot: \"$fhirRoot\"\n          targetAttribute: \"code\"\n          operator: \"one of\"\n          criteria: \"{code}\"\n        fhir:\n          - path: \"code\"\n            value: \"{code}\"\n        openehrCondition:\n          targetRoot: \"$archetype\"\n          targetAttribute: \"data[at0001]/items[at0073]/defining_code/code_string\"\n          operator: \"one of\"\n          criteria: \"{local}\"\n"
        )
    };
    let program = synthetic(&format!(
        "mappings:\n  - name: \"problemName\"\n    with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype/data[at0001]/items[at0002]\"\n      type: \"CODEABLECONCEPT\"\n  - name: \"status\"\n    with:\n      fhir: \"$resource.verificationStatus.coding\"\n      openehr: \"$archetype/data[at0001]/items[at0073]\"\n    manual:\n{}{}",
        entry("confirmed", "confirmed", "at-c"),
        entry("refuted", "refuted", "at-r")
    ))?;
    let index = template()?;
    let (composition, _) = inbound(
        &program,
        &index,
        &condition(&serde_json::json!({
            "verificationStatus": { "coding": [{ "code": "refuted" }] }
        })),
        &defaults(),
    )?;
    let back = outbound(&program, &index, &composition)?;
    assert_eq!(
        back["verificationStatus"]["coding"],
        serde_json::json!([{ "code": "refuted" }]),
        "only the entry whose openehrCondition holds wrote: {back}"
    );
    Ok(())
}

#[test]
fn a_required_structural_child_is_provided_by_a_value_below_it() -> Result<(), Box<dyn Error>> {
    let ran = kds_run(&kds_required(""), &condition(&serde_json::json!({})))?;
    assert!(
        ran.is_ok(),
        "the value below the entry provides it: {:?}",
        ran.err()
    );
    Ok(())
}

#[test]
fn a_required_structural_child_with_nothing_below_it_refuses() -> Result<(), Box<dyn Error>> {
    // engine/Fail.adoc: "the mapping should fail if the child is not
    // provided".
    let resource = Value::from_serde_json(serde_json::json!({ "resourceType": "Condition" }));
    let ran = kds_run(&kds_required(""), &resource)?;
    assert!(
        matches!(ran, Err(EngineError::MissingRequired { ref mapping, .. }) if mapping == "root.entry"),
        "{ran:?}"
    );
    Ok(())
}

#[test]
fn a_required_child_its_own_condition_closes_is_skipped() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc: a condition filters the input it is given, so an input
    // the condition turns away was provided and is none this mapping maps.
    let gated = "          fhirCondition:\n            targetRoot: \"$resource.code.coding\"\n            targetAttribute: \"system\"\n            operator: \"one of\"\n            criteria: \"http://example.org/fhir/CodeSystem/another-system\"\n";
    let ran = kds_run(&kds_required(gated), &condition(&serde_json::json!({})))?;
    assert!(
        ran.is_ok(),
        "the closed condition skips the child: {:?}",
        ran.err()
    );
    Ok(())
}
