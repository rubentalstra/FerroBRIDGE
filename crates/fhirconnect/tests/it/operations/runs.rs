// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The mapping runs through the two operations: the Bundle, the provenance,
//! the call context and the template pins.

use core::error::Error;

use fhir_types::codec::Json;
use fhir_types::codec::Path;
use fhir_types::r4::resource::Resource;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::traverse::functions::NoMappingFunctions;
use fhirconnect::operations::contract::CompositionPayload;
use fhirconnect::operations::contract::Format;
use fhirconnect::operations::contract::ToFhirRequest;
use fhirconnect::operations::contract::ToOpenehrRequest;
use fhirconnect::operations::error::OperationError;
use fhirconnect::operations::run;
use fhirconnect::resolve::program::binding::TemplateId;

use crate::operations::COMPOSITION_UID;
use crate::operations::DEVICE;
use crate::operations::MINIMAL_PROFILE;
use crate::operations::NOW;
use crate::operations::REQUIRED_PROFILE;
use crate::operations::SUBJECT_CONTEXT;
use crate::operations::SUBJECT_PROFILE;
use crate::operations::TEMPLATE;
use crate::operations::bundle;
use crate::operations::committed_composition;
use crate::operations::condition;
use crate::operations::program_set;
use crate::operations::reference;
use crate::operations::settings;

#[test]
fn a_split_answers_every_created_resource_as_an_entry() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §split: each anatomical-location cluster creates
    // its own Condition, and the one Provenance covers every one of them.
    let set = program_set(&["ferrobridge_split"])?;
    let mut resource = condition("http://example.org/fhir/StructureDefinition/ferrobridge-split")?;
    if let Some(object) = resource.as_object_mut() {
        object.insert(
            String::from("bodySite"),
            serde_json::json!([{"text": "Left knee"}, {"text": "Right knee"}]),
        );
    }
    let inbound = run::to_openehr(
        &set,
        &SCHEMAS,
        &NoMappingFunctions,
        &settings(),
        &ToOpenehrRequest::new(bundle(vec![resource])?),
    )?;
    let mut built: serde_json::Value = serde_json::from_str(inbound.composition())?;
    if let Some(object) = built.as_object_mut() {
        object.insert(
            String::from("uid"),
            serde_json::json!({ "_type": "OBJECT_VERSION_ID", "value": COMPOSITION_UID }),
        );
    }
    let request = ToFhirRequest::new(CompositionPayload::parse(&built.to_string())?);
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let conditions: Vec<String> = answer
        .bundle()
        .entry
        .iter()
        .filter_map(|entry| match entry.resource {
            Some(Resource::Condition(ref condition)) => condition.id.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(
        conditions.len(),
        2,
        "one Condition per cluster: {conditions:?}"
    );
    let targets = answer
        .bundle()
        .entry
        .iter()
        .find_map(|entry| match entry.resource {
            Some(Resource::Provenance(ref provenance)) => Some(provenance.target.len()),
            _ => None,
        })
        .ok_or("the Bundle carries the Provenance")?;
    assert_eq!(targets, 2, "the Provenance covers both Conditions");
    Ok(())
}

#[test]
fn the_diagnosis_chain_maps_both_ways_through_the_operations() -> Result<(), Box<dyn Error>> {
    // A canonical composition in, a Condition Bundle with a Provenance out;
    // that Bundle back in, the same composition out.
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?);
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let carried = answer.bundle();
    assert_eq!(carried.r#type.value.as_deref(), Some("collection"));
    let mapped = carried
        .entry
        .first()
        .and_then(|entry| entry.resource.as_ref())
        .ok_or("the Bundle carries the mapped resource first")?;
    let Resource::Condition(ref condition) = *mapped else {
        return Err(Box::<dyn Error>::from("the mapped resource is a Condition"));
    };
    assert_eq!(
        condition
            .code
            .as_ref()
            .and_then(|code| code.text.as_ref())
            .and_then(|text| text.value.clone()),
        Some(String::from("Synthetic problem one")),
        "the problem name survived the round trip"
    );
    let back = ToOpenehrRequest::new(bundle(vec![
        fhir_types::codec::Value::Object(Json::to_json(mapped)?)
            .to_serde_json(&mut Path::root("Condition"))?,
    ])?)
    .with_template_id(TemplateId::new(TEMPLATE));
    let second = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &back)?;
    let mut reread: serde_json::Value = serde_json::from_str(second.composition())?;
    // The FEEDER_AUDIT names the resource each pass read (RM 1.1.0
    // common.html §FEEDER_AUDIT), and the second pass reads the Condition
    // `$tofhir` identified, so the audit is asserted on its own and the
    // content is compared without it.
    let audit = reread
        .as_object_mut()
        .and_then(|object| object.remove("feeder_audit"))
        .ok_or("the second pass records its origin")?;
    assert_eq!(
        audit
            .pointer("/originating_system_item_ids/0/id")
            .and_then(serde_json::Value::as_str),
        condition.id.as_deref(),
        "the second pass records the Condition it read: {audit}"
    );
    let first_pass: serde_json::Value = {
        let mut without = committed.clone();
        if let Some(object) = without.as_object_mut() {
            object.remove("uid");
            object.remove("feeder_audit");
        }
        without
    };
    assert_eq!(
        reread, first_pass,
        "the second pass produced another composition"
    );
    Ok(())
}

#[test]
fn every_tofhir_bundle_carries_one_provenance_covering_every_entry() -> Result<(), Box<dyn Error>> {
    // "The engine generates a Provenance resource as a side-effect of every
    // run ... and includes it in the Bundle" (rest-api.adoc, draft, section
    // $tofhir Output).
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?);
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let provenances: Vec<&fhir_types::r4::provenance::Provenance> = answer
        .bundle()
        .entry
        .iter()
        .filter_map(|entry| match entry.resource {
            Some(Resource::Provenance(ref carried)) => Some(carried.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(provenances.len(), 1, "a run carries exactly one Provenance");
    let provenance = provenances.first().ok_or("one Provenance")?;
    let mapped: Vec<String> = answer
        .bundle()
        .entry
        .iter()
        .filter_map(|entry| match entry.resource {
            Some(Resource::Condition(ref condition)) => {
                condition.id.as_ref().map(|id| format!("Condition/{id}"))
            }
            _ => None,
        })
        .collect();
    let targets: Vec<String> = provenance
        .target
        .iter()
        .filter_map(|target| target.reference.as_ref())
        .filter_map(|value| value.value.clone())
        .collect();
    assert_eq!(targets, mapped, "the targets cover every mapped entry");
    assert_eq!(
        provenance.recorded.value.as_deref(),
        Some(NOW),
        "recorded is the run time"
    );
    assert_eq!(
        provenance
            .agent
            .first()
            .and_then(|agent| agent.who.reference.as_ref())
            .and_then(|value| value.value.clone()),
        Some(String::from(DEVICE)),
        "a call with no context.who gets the configured device"
    );
    assert_eq!(
        provenance
            .entity
            .first()
            .map(|entity| entity.role.value.clone()),
        Some(Some(String::from("derivation")))
    );
    Ok(())
}

#[test]
fn the_call_context_overrides_the_provenance_agent() -> Result<(), Box<dyn Error>> {
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?)
        .with_context(
            CallContext::new()
                .with_who(reference("Practitioner/456"))
                .with_on_behalf_of(reference("Organization/charite")),
        );
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let provenance = answer
        .bundle()
        .entry
        .iter()
        .find_map(|entry| match entry.resource {
            Some(Resource::Provenance(ref carried)) => Some(carried.as_ref()),
            _ => None,
        })
        .ok_or("the Bundle carries a Provenance")?;
    let agent = provenance.agent.first().ok_or("one agent")?;
    assert_eq!(
        agent.who.reference.as_ref().and_then(|v| v.value.clone()),
        Some(String::from("Practitioner/456"))
    );
    assert_eq!(
        agent
            .on_behalf_of
            .as_ref()
            .and_then(|value| value.reference.as_ref())
            .and_then(|value| value.value.clone()),
        Some(String::from("Organization/charite"))
    );
    Ok(())
}

#[test]
fn the_context_patient_takes_precedence_over_the_mapped_subject() -> Result<(), Box<dyn Error>> {
    // "When the engine can resolve the subject itself and `context.patient` is
    // also supplied, the supplied value takes precedence" (rest-api.adoc,
    // draft, section Resolving the patient).
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?)
        .with_context(CallContext::new().with_patient(reference("Patient/123")));
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let condition = answer
        .bundle()
        .entry
        .iter()
        .find_map(|entry| match entry.resource {
            Some(Resource::Condition(ref carried)) => Some(carried.as_ref()),
            _ => None,
        })
        .ok_or("the Bundle carries the mapped Condition")?;
    assert_eq!(
        condition
            .subject
            .reference
            .as_ref()
            .and_then(|value| value.value.clone()),
        Some(String::from("Patient/123"))
    );
    Ok(())
}

#[test]
fn a_call_that_omits_the_context_patient_still_succeeds() -> Result<(), Box<dyn Error>> {
    // "An engine MUST NOT require `context.patient`" (rest-api.adoc, draft,
    // section Resolving the patient).
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?);
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let condition = answer
        .bundle()
        .entry
        .iter()
        .find_map(|entry| match entry.resource {
            Some(Resource::Condition(ref carried)) => Some(carried.as_ref()),
            _ => None,
        })
        .ok_or("the Bundle carries the mapped Condition")?;
    assert_eq!(
        condition
            .subject
            .reference
            .as_ref()
            .and_then(|value| value.value.clone()),
        Some(String::from("Patient/synthetic-subject-0001")),
        "the subject the chain resolved on its own survived"
    );
    Ok(())
}

#[test]
fn a_context_member_the_caller_supplied_reaches_the_mapping() -> Result<(), Box<dyn Error>> {
    // "`$context` holds values passed in on the REST call ... a context value
    // is referenced from a `manual` `value`" (basics/Variables.adoc).
    let set = program_set(&["ferrobridge_diagnose_context"])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?)
        .with_template_id(TemplateId::new(TEMPLATE))
        .with_context(CallContext::new().with_who(reference("Practitioner/456")));
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let condition = answer
        .bundle()
        .entry
        .iter()
        .find_map(|entry| match entry.resource {
            Some(Resource::Condition(ref carried)) => Some(carried.as_ref()),
            _ => None,
        })
        .ok_or("the Bundle carries the mapped Condition")?;
    assert_eq!(
        condition
            .asserter
            .as_ref()
            .and_then(|value| value.reference.as_ref())
            .and_then(|value| value.value.clone()),
        Some(String::from("Practitioner/456"))
    );
    Ok(())
}

#[test]
fn a_context_member_the_caller_did_not_supply_is_a_refusal() -> Result<(), Box<dyn Error>> {
    // A `$context` member is a per-call value, so naming one the call does not
    // carry is a refusal rather than an invented value.
    let set = program_set(&["ferrobridge_diagnose_context"])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?)
        .with_template_id(TemplateId::new(TEMPLATE));
    let error = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("the mapping names a member the call does not carry")?;
    let rendered = error.to_string();
    assert!(
        matches!(error, OperationError::Mapping { .. }),
        "{rendered}"
    );
    assert!(
        error.outcome().issue.first().is_some_and(|issue| issue
            .details
            .as_ref()
            .and_then(|details| details.text.as_ref())
            .and_then(|text| text.value.as_deref())
            .is_some_and(|text| text.contains("$context.who"))),
        "the outcome names the member the call did not carry"
    );
    Ok(())
}

#[test]
fn a_bundle_referencing_two_subjects_is_refused_naming_them() -> Result<(), Box<dyn Error>> {
    let set = program_set(&["ferrobridge_diagnose_minimal"])?;
    let mut first = condition(MINIMAL_PROFILE)?;
    if let Some(object) = first.as_object_mut() {
        object.insert(
            String::from("subject"),
            serde_json::json!({ "reference": "Patient/one" }),
        );
    }
    let mut second = condition(MINIMAL_PROFILE)?;
    if let Some(object) = second.as_object_mut() {
        object.insert(String::from("id"), serde_json::json!("another"));
        object.insert(
            String::from("subject"),
            serde_json::json!({ "reference": "Patient/two" }),
        );
    }
    let request = ToOpenehrRequest::new(bundle(vec![first, second])?);
    let error = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("a mixed-subject bundle cannot be one composition")?;
    let rendered = error.to_string();
    assert!(
        rendered.contains("Patient/one") && rendered.contains("Patient/two"),
        "the refusal names the conflicting subjects: {rendered}"
    );
    assert_eq!(error.issue_type(), "business-rule");
    Ok(())
}

#[test]
fn a_failed_mapping_answers_an_outcome_and_nothing_else() -> Result<(), Box<dyn Error>> {
    // Fail.adoc: a required child the input does not carry refuses the unit,
    // and FerroBRIDGE answers no partial composition (architecture section
    // 4.7).
    let set = program_set(&["ferrobridge_diagnose_required"])?;
    let mut without_code = condition(REQUIRED_PROFILE)?;
    if let Some(object) = without_code.as_object_mut() {
        object.remove("code");
    }
    let request = ToOpenehrRequest::new(bundle(vec![without_code])?);
    let error = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("the required child is not provided")?;
    assert!(matches!(error, OperationError::Mapping { .. }));
    let outcome = error.outcome();
    assert_eq!(outcome.issue.len(), 1);
    assert_eq!(
        outcome
            .issue
            .first()
            .and_then(|issue| issue.severity.value.clone()),
        Some(String::from("error")),
        "a refusal is an error, never a warning beside a partial result"
    );
    Ok(())
}

#[test]
fn a_disagreeing_template_pin_is_refused() -> Result<(), Box<dyn Error>> {
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?)
        .with_template_id(TemplateId::new("another.template.v1"));
    let error = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("a pin that contradicts the payload is refused")?;
    assert!(matches!(error, OperationError::TemplateDisagreement { .. }));
    assert_eq!(error.issue_type(), "invalid");
    Ok(())
}

#[test]
fn an_agreeing_template_pin_is_accepted() -> Result<(), Box<dyn Error>> {
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?)
        .with_template_id(TemplateId::new(TEMPLATE));
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    assert!(!answer.bundle().entry.is_empty());
    Ok(())
}

#[test]
fn a_flat_composition_maps_in_and_a_flat_composition_maps_out() -> Result<(), Box<dyn Error>> {
    // "An engine MUST support both, and the returned Composition is the same
    // content either way" (rest-api.adoc, draft, section $toopenehr Output).
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let request =
        ToOpenehrRequest::new(bundle(vec![condition(SUBJECT_PROFILE)?])?).with_format(Format::Flat);
    let answer = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let flat: serde_json::Value = serde_json::from_str(answer.composition())?;
    let members = flat.as_object().ok_or("a flat document is an object")?;
    assert!(
        members.keys().any(|key| key.contains('/')),
        "a flat document keys its values by template path"
    );
    let inbound = ToFhirRequest::new(CompositionPayload::parse(answer.composition())?)
        .with_template_id(TemplateId::new(TEMPLATE));
    assert!(
        matches!(*inbound.composition(), CompositionPayload::Flat(_)),
        "the flat answer reads back as the flat serialization"
    );
    let error = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &inbound)
        .err()
        .ok_or("a flat payload carries no uid, so the Provenance has no target")?;
    assert!(matches!(error, OperationError::Unidentified { .. }));
    Ok(())
}

#[test]
fn a_composition_with_no_identity_is_refused() -> Result<(), Box<dyn Error>> {
    // The draft puts a Provenance in every $tofhir Bundle and
    // Provenance.target is a reference, so a resource with no identity is a
    // refusal rather than a Bundle with an empty target.
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let mut committed = committed_composition()?;
    if let Some(object) = committed.as_object_mut() {
        object.remove("uid");
    }
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?);
    let error = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("a composition with no uid gives the Provenance no target")?;
    assert!(matches!(error, OperationError::Unidentified { .. }));
    Ok(())
}

#[test]
fn the_mapped_resource_takes_the_composition_version_object_id() -> Result<(), Box<dyn Error>> {
    // The versioned_object_uid is the one part of an OBJECT_VERSION_ID stable
    // across versions (RM Common), and it fits the R4 id grammar
    // (<https://hl7.org/fhir/R4/resource.html>).
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let request = ToFhirRequest::new(CompositionPayload::parse(&committed.to_string())?);
    let answer = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let condition = answer
        .bundle()
        .entry
        .iter()
        .find_map(|entry| match entry.resource {
            Some(Resource::Condition(ref carried)) => Some(carried.as_ref()),
            _ => None,
        })
        .ok_or("the Bundle carries the mapped Condition")?;
    assert_eq!(
        condition.id.as_deref(),
        Some("d2b3c1a0-0000-4000-8000-000000000001")
    );
    Ok(())
}

#[test]
fn a_template_the_set_does_not_hold_is_refused() -> Result<(), Box<dyn Error>> {
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let committed = committed_composition()?;
    let mut other = committed;
    if let Some(details) = other
        .get_mut("archetype_details")
        .and_then(serde_json::Value::as_object_mut)
    {
        details.insert(
            String::from("template_id"),
            serde_json::json!({ "value": "not.loaded.v1" }),
        );
    }
    let request = ToFhirRequest::new(CompositionPayload::parse(&other.to_string())?);
    let error = run::to_fhir(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("a template the set does not hold is refused")?;
    assert!(matches!(error, OperationError::UnknownTemplate { .. }));
    assert_eq!(error.issue_type(), "not-found");
    Ok(())
}

#[test]
fn a_bundle_claiming_no_known_profile_is_refused() -> Result<(), Box<dyn Error>> {
    let set = program_set(&["ferrobridge_diagnose_minimal"])?;
    let request = ToOpenehrRequest::new(bundle(vec![condition(
        "http://example.org/fhir/StructureDefinition/unknown",
    )?])?);
    let error = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)
        .err()
        .ok_or("no program claims that profile")?;
    assert!(matches!(error, OperationError::Select { .. }));
    assert_eq!(error.issue_type(), "not-found");
    Ok(())
}

#[test]
fn a_template_pin_narrows_two_contexts_over_one_profile() -> Result<(), Box<dyn Error>> {
    // "supplying `templateId` pin-points exactly which context mapping to
    // apply" (rest-api.adoc, draft, section Query parameters).
    let set = program_set(&[
        "ferrobridge_diagnose_minimal",
        "ferrobridge_diagnose_required",
    ])?;
    let request = ToOpenehrRequest::new(bundle(vec![condition(REQUIRED_PROFILE)?])?)
        .with_template_id(TemplateId::new(TEMPLATE));
    let answer = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    assert!(
        answer.composition().contains("Synthetic problem one"),
        "the pinned context mapped the problem name"
    );
    Ok(())
}
