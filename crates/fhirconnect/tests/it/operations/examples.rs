// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The worked examples of the draft chapter and the refusals of a malformed
//! parameter set.

use core::error::Error;

use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::parameters::ParametersParameter;
use fhir_types::r4::parameters::ParametersParameterValue;
use fhir_types::r4::primitives::Code;
use fhirconnect::engine::context::CallContext;
use fhirconnect::operations::contract::CompositionPayload;
use fhirconnect::operations::contract::ToFhirRequest;
use fhirconnect::operations::contract::ToOpenehrResponse;
use fhirconnect::operations::error::OperationError;

use crate::operations::draft;
use crate::operations::normalized;
use crate::operations::parameters;
use crate::operations::rendered;

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
