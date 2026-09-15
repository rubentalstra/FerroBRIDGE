// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The two FHIRconnect operations, against the DRAFT REST API chapter.
//!
//! Every wire case here reads the chapter at its pinned commit, an unmerged
//! draft (pull request #93 of the specification), so the fixtures under
//! `tests/fixtures/draft-rest-api/` are the chapter's own worked examples and
//! the expectations move with it when it merges.

use core::error::Error;

use fhir_types::codec::Json;
use fhir_types::codec::Path;
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::bundle::BundleEntry;
use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::parameters::ParametersParameter;
use fhir_types::r4::parameters::ParametersParameterValue;
use fhir_types::r4::primitives::Code;
use fhir_types::r4::reference::Reference;
use fhir_types::r4::resource::Resource;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::traverse::NoMappingFunctions;
use fhirconnect::operations::contract::CompositionPayload;
use fhirconnect::operations::contract::Format;
use fhirconnect::operations::contract::ToFhirRequest;
use fhirconnect::operations::contract::ToOpenehrRequest;
use fhirconnect::operations::contract::ToOpenehrResponse;
use fhirconnect::operations::error::OperationError;
use fhirconnect::operations::programs::ProgramSet;
use fhirconnect::operations::run;
use fhirconnect::operations::run::Settings;
use fhirconnect::resolve::program::TemplateId;

use crate::support::FIXTURES;
use crate::support::compiled;
use crate::support::template;

/// The profile the minimal diagnosis context claims.
const MINIMAL_PROFILE: &str =
    "http://example.org/fhir/StructureDefinition/ferrobridge-diagnose-minimal";

/// The profile the required-child diagnosis context claims.
const REQUIRED_PROFILE: &str =
    "http://example.org/fhir/StructureDefinition/ferrobridge-diagnose-required";

/// The profile the subject-resolving diagnosis context claims.
const SUBJECT_PROFILE: &str =
    "http://example.org/fhir/StructureDefinition/ferrobridge-diagnose-subject";

/// The context mapping whose chain resolves the subject on its own.
const SUBJECT_CONTEXT: &str = "ferrobridge_diagnose_subject";

/// The template both contexts compile against.
const TEMPLATE: &str = "ferrobridge.diagnose.v1";

/// A synthetic `OBJECT_VERSION_ID` for a composition a CDR would have stamped.
const COMPOSITION_UID: &str = "d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1";

/// The run timestamp the tests pin, so nothing reads a clock.
const NOW: &str = "2026-09-15T10:00:00Z";

/// The `Device` reference a call that supplies no `context.who` gets.
const DEVICE: &str = "Device/ferrobridge";

/// Returns the text of one vendored draft example.
fn draft(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(std::fs::read_to_string(format!(
        "{FIXTURES}/draft-rest-api/{name}"
    ))?)
}

/// Reads JSON text as a `Parameters` resource.
fn parameters(text: &str) -> Result<Parameters, Box<dyn Error>> {
    let parsed: serde_json::Value = serde_json::from_str(text)?;
    let fhir_types::codec::Value::Object(object) =
        fhir_types::codec::Value::from_serde_json(parsed)
    else {
        return Err(Box::<dyn Error>::from("the example is not a JSON object"));
    };
    Ok(Parameters::from_json(
        &object,
        &mut Path::root("Parameters"),
    )?)
}

/// Renders a `Parameters` resource as plain JSON.
fn rendered(parameters: &Parameters) -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(fhir_types::codec::Value::Object(Json::to_json(parameters)?)
        .to_serde_json(&mut Path::root("Parameters"))?)
}

/// Returns `document` with every `composition` string parsed in place.
///
/// The composition travels as an opaque JSON string, so comparing two
/// renderings byte for byte would compare their whitespace and key order too.
/// Parsing the string first is what "modulo whitespace and key order" means.
fn normalized(mut document: serde_json::Value) -> serde_json::Value {
    let Some(list) = document
        .get_mut("parameter")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return document;
    };
    for parameter in list.iter_mut() {
        if parameter.get("name").and_then(serde_json::Value::as_str) != Some("composition") {
            continue;
        }
        let Some(text) = parameter
            .get("valueString")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) else {
            continue;
        };
        if let Some(object) = parameter.as_object_mut() {
            object.insert(String::from("valueString"), parsed);
        }
    }
    document
}

/// Builds the compiled set over the fixtures `contexts` names.
fn program_set(contexts: &[&str]) -> Result<ProgramSet, Box<dyn Error>> {
    let mut set = ProgramSet::new();
    set.insert_template(template()?);
    for context in contexts {
        set.insert_program(compiled(context)?)?;
    }
    Ok(set)
}

/// Returns the settings the tests run under.
///
/// `defaults-for-fields.adoc` puts the composer and the context start time on
/// the engine and the composition language and territory on "the project
/// performing the mapping", so they are settings rather than constants.
fn settings() -> Settings {
    Settings::new(DEVICE, NOW).with_defaults(
        fhirconnect::engine::traverse::Defaults::at(NOW)
            .with_language("en")
            .with_territory("NL"),
    )
}

/// Returns the synthetic `Condition` of the testkit, claiming `profile`.
fn condition(profile: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("meta"),
            serde_json::json!({ "profile": [profile] }),
        );
    }
    Ok(parsed)
}

/// Returns a `collection` Bundle carrying `resources`.
fn bundle(resources: Vec<serde_json::Value>) -> Result<Bundle, Box<dyn Error>> {
    let mut entry = Vec::new();
    for resource in resources {
        let fhir_types::codec::Value::Object(object) =
            fhir_types::codec::Value::from_serde_json(resource)
        else {
            return Err(Box::<dyn Error>::from("a resource is not a JSON object"));
        };
        let name = object
            .get("resourceType")
            .and_then(fhir_types::codec::Value::as_str)
            .unwrap_or("Resource")
            .to_owned();
        entry.push(BundleEntry {
            resource: Some(Resource::from_json(&object, &mut Path::root(&name))?),
            ..BundleEntry::default()
        });
    }
    Ok(Bundle {
        r#type: Code::from("collection"),
        entry,
        ..Bundle::default()
    })
}

/// Returns a `Reference` whose literal reference is `text`.
fn reference(text: &str) -> Reference {
    Reference {
        reference: Some(fhir_types::r4::primitives::String::from(text)),
        ..Reference::default()
    }
}

/// Maps the synthetic `Condition` into a composition and stamps a `uid` on it.
///
/// A composition a CDR served always carries its `OBJECT_VERSION_ID`; the
/// engine's builder stamps none, so the test stamps the one a commit would
/// have written.
fn committed_composition() -> Result<serde_json::Value, Box<dyn Error>> {
    let set = program_set(&[SUBJECT_CONTEXT])?;
    let request = ToOpenehrRequest::new(bundle(vec![condition(SUBJECT_PROFILE)?])?);
    let answer = run::to_openehr(&set, &SCHEMAS, &NoMappingFunctions, &settings(), &request)?;
    let mut built: serde_json::Value = serde_json::from_str(answer.composition())?;
    if let Some(object) = built.as_object_mut() {
        object.insert(
            String::from("uid"),
            serde_json::json!({ "_type": "OBJECT_VERSION_ID", "value": COMPOSITION_UID }),
        );
    }
    Ok(built)
}

#[test]
fn the_draft_canonical_tofhir_example_round_trips() -> Result<(), Box<dyn Error>> {
    // rest-api.adoc (draft) section $tofhir Input, `.Canonical Composition`.
    let text = draft("tofhir-canonical.request.json")?;
    let request = ToFhirRequest::from_parameters(&parameters(&text)?)?;
    assert_eq!(
        request
            .composition()
            .template_id()
            .map(|template| String::from(template.as_str())),
        Some(String::from("KDS_Fall_einfach")),
        "the canonical payload carries its template inline"
    );
    assert_eq!(
        request.context().and_then(CallContext::ehr_id),
        Some("53d89df2-5501-4455-9a65-565a5d1ddb7c")
    );
    let written = rendered(&request.to_parameters())?;
    assert_eq!(
        normalized(written),
        normalized(serde_json::from_str(&text)?),
        "the request does not round trip through the Parameters envelope"
    );
    Ok(())
}

#[test]
fn the_draft_flat_tofhir_example_round_trips() -> Result<(), Box<dyn Error>> {
    // rest-api.adoc (draft) section $tofhir Input, `.Flat Composition`.
    let text = draft("tofhir-flat.request.json")?;
    let request = ToFhirRequest::from_parameters(&parameters(&text)?)?;
    assert!(
        matches!(*request.composition(), CompositionPayload::Flat(_)),
        "a payload with no `_type` is the flat serialization"
    );
    assert_eq!(
        request
            .template_id()
            .map(|template| String::from(template.as_str())),
        Some(String::from("Blood Pressure")),
        "the flat payload is accompanied by its template"
    );
    let written = rendered(&request.to_parameters())?;
    assert_eq!(
        normalized(written),
        normalized(serde_json::from_str(&text)?)
    );
    Ok(())
}

#[test]
fn the_draft_toopenehr_responses_round_trip() -> Result<(), Box<dyn Error>> {
    // rest-api.adoc (draft) section $toopenehr Output: the two serializations
    // and the partial-result shape.
    for name in [
        "toopenehr-canonical.response.json",
        "toopenehr-flat.response.json",
        "toopenehr-outcome.response.json",
    ] {
        let text = draft(name)?;
        let answer = ToOpenehrResponse::from_parameters(&parameters(&text)?)?;
        let written = rendered(&answer.to_parameters())?;
        assert_eq!(
            written,
            serde_json::from_str::<serde_json::Value>(&text)?,
            "{name} does not round trip"
        );
    }
    Ok(())
}

#[test]
fn the_partial_result_example_carries_its_outcome() -> Result<(), Box<dyn Error>> {
    let text = draft("toopenehr-outcome.response.json")?;
    let answer = ToOpenehrResponse::from_parameters(&parameters(&text)?)?;
    let outcome = answer.outcome().ok_or("the example carries an outcome")?;
    assert_eq!(
        outcome
            .issue
            .first()
            .and_then(|issue| issue.code.value.clone()),
        Some(String::from("incomplete"))
    );
    Ok(())
}

#[test]
fn a_flat_composition_without_a_template_is_required() -> Result<(), Box<dyn Error>> {
    // "`templateId` ... is also required when submitting a flat Composition to
    // `$tofhir`" (rest-api.adoc, draft, section Query parameters).
    let text = draft("tofhir-flat.request.json")?;
    let mut carried = parameters(&text)?;
    carried
        .parameter
        .retain(|parameter| parameter.name.value.as_deref() != Some("templateId"));
    let error = ToFhirRequest::from_parameters(&carried)
        .err()
        .ok_or("a flat payload with no templateId is refused")?;
    assert!(matches!(error, OperationError::FlatTemplate));
    assert_eq!(error.issue_type(), "required");
    Ok(())
}

#[test]
fn an_undeclared_parameter_is_refused() -> Result<(), Box<dyn Error>> {
    let text = draft("tofhir-canonical.request.json")?;
    let mut carried = parameters(&text)?;
    carried.parameter.push(ParametersParameter {
        name: fhir_types::r4::primitives::String::from("subject"),
        value: Some(ParametersParameterValue::String(
            fhir_types::r4::primitives::String::from("Patient/123"),
        )),
        ..ParametersParameter::default()
    });
    let error = ToFhirRequest::from_parameters(&carried)
        .err()
        .ok_or("a parameter the operation does not declare is refused")?;
    assert_eq!(error.issue_type(), "structure");
    assert!(
        error.to_string().contains("declared parameters"),
        "the refusal says the parameter set was not met: {error}"
    );
    Ok(())
}

#[test]
fn a_repeated_parameter_is_refused() -> Result<(), Box<dyn Error>> {
    let text = draft("tofhir-canonical.request.json")?;
    let mut carried = parameters(&text)?;
    let first = carried
        .parameter
        .first()
        .cloned()
        .ok_or("the example carries the composition")?;
    carried.parameter.push(first);
    let error = ToFhirRequest::from_parameters(&carried)
        .err()
        .ok_or("a parameter with a maximum of one is refused twice")?;
    assert_eq!(error.issue_type(), "structure");
    Ok(())
}

#[test]
fn a_wrongly_typed_parameter_is_refused() -> Result<(), Box<dyn Error>> {
    let mut carried = Parameters::default();
    carried.parameter.push(ParametersParameter {
        name: fhir_types::r4::primitives::String::from("composition"),
        value: Some(ParametersParameterValue::Code(Code::from("a-code"))),
        ..ParametersParameter::default()
    });
    let error = ToFhirRequest::from_parameters(&carried)
        .err()
        .ok_or("a composition that is not a string is refused")?;
    assert_eq!(error.issue_type(), "value");
    Ok(())
}

#[test]
fn an_undeclared_context_part_is_refused() -> Result<(), Box<dyn Error>> {
    let text = draft("tofhir-canonical.request.json")?;
    let mut carried = parameters(&text)?;
    for parameter in &mut carried.parameter {
        if parameter.name.value.as_deref() == Some("context") {
            parameter.part.push(ParametersParameter {
                name: fhir_types::r4::primitives::String::from("composer"),
                value: Some(ParametersParameterValue::String(
                    fhir_types::r4::primitives::String::from("Dr Synthetic"),
                )),
                ..ParametersParameter::default()
            });
        }
    }
    let error = ToFhirRequest::from_parameters(&carried)
        .err()
        .ok_or("a context part the draft does not declare is refused")?;
    assert_eq!(error.issue_type(), "structure");
    Ok(())
}

#[test]
fn a_missing_composition_is_required() -> Result<(), Box<dyn Error>> {
    let error = ToFhirRequest::from_parameters(&Parameters::default())
        .err()
        .ok_or("`composition` has a minimum of one")?;
    assert_eq!(error.issue_type(), "required");
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
    let reread: serde_json::Value = serde_json::from_str(second.composition())?;
    let first_pass: serde_json::Value = {
        let mut without = committed.clone();
        if let Some(object) = without.as_object_mut() {
            object.remove("uid");
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
