// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The resolver of an expression over the R4 element table.

use fhir_types::r4::schema::SCHEMAS;
use fhir_types::schema::Kind;
use fhir_types::schema::ValueKind;

use fhirconnect::tree::element::Location;
use fhirconnect::tree::element::Move;
use fhirconnect::tree::element::Resolved;
use fhirconnect::tree::element::resolve;
use fhirconnect::tree::error::ParseError;
use fhirconnect::tree::error::ResolveError;

fn resolved(resource: &str, expression: &str) -> Result<Resolved, Box<dyn core::error::Error>> {
    let path = expression.parse()?;
    Ok(resolve(&SCHEMAS, resource, &path)?)
}

fn keys(resolved: &Resolved) -> Vec<String> {
    resolved
        .moves()
        .iter()
        .filter_map(|step| match step {
            Move::Member(field) | Move::Extension { field, .. } | Move::Choice { field, .. } => {
                Some(String::from(field.key()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_choice_resolves_to_the_suffixed_json_key() -> Result<(), Box<dyn core::error::Error>> {
    for expression in [
        "$resource.onset.ofType(Period)",
        "$resource.onset.as(Period)",
        "$resource.onsetPeriod",
    ] {
        let resolved = resolved("Condition", expression)?;
        assert_eq!(keys(&resolved), ["onsetPeriod"], "{expression}");
        assert_eq!(resolved.leaf(), "Condition.onset[x]", "{expression}");
    }
    Ok(())
}

#[test]
fn a_type_the_choice_does_not_admit_names_the_element_and_the_alternatives()
-> Result<(), ParseError> {
    let path = "$resource.onset.ofType(Quantity)".parse()?;
    let refused = resolve(&SCHEMAS, "Condition", &path);
    let Err(ResolveError::ChoiceType {
        element,
        requested,
        admitted,
    }) = refused
    else {
        panic!("a wrong choice type should have been refused")
    };
    assert_eq!(element, "Condition.onset[x]");
    assert_eq!(requested, "Quantity");
    assert_eq!(admitted, "DateTime, Age, Period, Range, String");
    Ok(())
}

#[test]
fn an_unknown_element_names_the_type_that_defines_none() -> Result<(), ParseError> {
    let path = "$resource.onsettt".parse()?;
    let refused = resolve(&SCHEMAS, "Condition", &path);
    assert!(matches!(
        refused,
        Err(ResolveError::UnknownElement { ref owner, ref name })
            if owner == "Condition" && name == "onsettt"
    ));
    Ok(())
}

#[test]
fn a_step_through_an_unresolved_choice_is_refused() -> Result<(), ParseError> {
    let path = "$resource.onset.start".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &path),
        Err(ResolveError::UnresolvedChoice { ref element }) if element == "Condition.onset[x]"
    ));
    Ok(())
}

#[test]
fn a_primitive_carries_its_extension_in_the_underscore_sibling()
-> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved(
        "Condition",
        "$resource.onset.ofType(Period).start.extension",
    )?;
    assert_eq!(keys(&resolved), ["onsetPeriod", "_start", "extension"]);
    assert_eq!(resolved.leaf(), "Period.start.extension");
    assert_eq!(
        resolved.location(),
        &Location::Complex(SCHEMAS.type_named("Extension").ok_or("no Extension type")?)
    );
    Ok(())
}

#[test]
fn the_underscore_sibling_reads_its_members_from_the_element_table()
-> Result<(), Box<dyn core::error::Error>> {
    let element = SCHEMAS.type_named("Element").ok_or("no Element type")?;
    let id = resolved("Condition", "$resource.recordedDate.id")?;
    assert_eq!(keys(&id), ["_recordedDate", "id"]);
    assert_eq!(id.leaf(), "Condition.recordedDate.id");
    assert_eq!(id.location(), &Location::Attribute);
    let extension = resolved("Condition", "$resource.recordedDate.extension")?;
    assert_eq!(keys(&extension), ["_recordedDate", "extension"]);
    let Some(Move::Member(field)) = extension.moves().last() else {
        panic!("the last move should be `extension`")
    };
    assert_eq!(
        (field.kind(), field.min(), field.max(), field.types()),
        element
            .fields
            .iter()
            .find(|candidate| candidate.name == "extension")
            .map(|candidate| (
                candidate.kind,
                candidate.min,
                candidate.max,
                candidate.types
            ))
            .ok_or("Element states no extension")?
    );
    Ok(())
}

#[test]
fn a_member_the_element_table_does_not_state_is_refused() -> Result<(), ParseError> {
    let path = "$resource.recordedDate.value".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &path),
        Err(ResolveError::NoChildren { ref element, ref name })
            if element == "Condition.recordedDate" && name == "value"
    ));
    Ok(())
}

#[test]
fn the_extension_shortcut_reaches_the_same_element() -> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved(
        "Condition",
        "$resource.extension('http://hl7.org/fhir/StructureDefinition/condition-assertedDate')",
    )?;
    assert_eq!(keys(&resolved), ["extension"]);
    let Some(Move::Extension { url, .. }) = resolved.moves().last() else {
        panic!("the shortcut should have resolved to an extension move")
    };
    assert_eq!(
        url,
        "http://hl7.org/fhir/StructureDefinition/condition-assertedDate"
    );
    Ok(())
}

#[test]
fn a_content_reference_lands_on_the_type_it_names() -> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved("Bundle", "$resource.entry.link.relation")?;
    assert_eq!(resolved.leaf(), "Bundle.link.relation");
    assert_eq!(resolved.location(), &Location::Primitive(ValueKind::Text));
    Ok(())
}

#[test]
fn a_resource_type_on_a_reference_defers_to_the_engine() -> Result<(), Box<dyn core::error::Error>>
{
    for expression in [
        "$resource.recorder.resolve().as(Practitioner).name",
        "$resource.recorder.as(Practitioner).name",
    ] {
        let resolved = resolved("Condition", expression)?;
        assert_eq!(resolved.location(), &Location::Deferred, "{expression}");
        let Some(Move::Resolve {
            expected,
            continuation,
        }) = resolved.moves().last()
        else {
            panic!("`{expression}` should have deferred")
        };
        assert_eq!(expected.as_deref(), Some("Practitioner"), "{expression}");
        assert_eq!(continuation.len(), 1, "{expression}");
    }
    Ok(())
}

#[test]
fn a_type_filter_on_the_element_s_own_type_is_an_assertion()
-> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved(
        "Condition",
        "$resource.encounter.ofType(Reference).identifier",
    )?;
    assert_eq!(keys(&resolved), ["encounter", "identifier"]);
    assert_eq!(resolved.leaf(), "Reference.identifier");
    Ok(())
}

#[test]
fn a_type_filter_that_is_not_the_element_s_type_names_both() -> Result<(), ParseError> {
    let path = "$resource.code.ofType(Quantity)".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &path),
        Err(ResolveError::TypeAssertion { ref requested, ref actual, .. })
            if requested == "Quantity" && actual == "CodeableConcept"
    ));
    Ok(())
}

#[test]
fn a_type_filter_on_a_primitive_reads_the_definition_s_own_type_code()
-> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved("Condition", "$resource.recordedDate.as(DateTime)")?;
    assert_eq!(keys(&resolved), ["recordedDate"]);
    let refused = "$resource.recordedDate.as(Period)".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &refused),
        Err(ResolveError::TypeAssertion { ref actual, .. }) if actual == "dateTime"
    ));
    Ok(())
}

#[test]
fn resolve_on_an_element_that_is_no_reference_is_refused() -> Result<(), ParseError> {
    let path = "$resource.recordedDate.resolve()".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &path),
        Err(ResolveError::NotAReference { ref element })
            if element == "Condition.recordedDate"
    ));
    Ok(())
}

#[test]
fn resolve_on_a_canonical_primitive_defers() -> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved("Condition", "$resource.meta.profile.resolve()")?;
    assert_eq!(resolved.location(), &Location::Deferred);
    Ok(())
}

#[test]
fn an_index_on_an_element_that_does_not_repeat_is_refused() -> Result<(), ParseError> {
    let path = "$resource.subject[0]".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &path),
        Err(ResolveError::NotRepeating { ref element, index: 0 })
            if element == "Condition.subject"
    ));
    Ok(())
}

#[test]
fn an_unbound_expression_is_refused_before_the_table_is_touched() -> Result<(), ParseError> {
    let path = "code.coding".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Condition", &path),
        Err(ResolveError::NotAnchored { .. })
    ));
    Ok(())
}

#[test]
fn an_unknown_resource_type_is_refused() -> Result<(), ParseError> {
    let path = "$resource.code".parse()?;
    assert!(matches!(
        resolve(&SCHEMAS, "Cndition", &path),
        Err(ResolveError::UnknownResource { ref name }) if name == "Cndition"
    ));
    Ok(())
}

#[test]
fn a_repeating_element_is_read_from_the_table() -> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved("Condition", "$resource.category.coding.code")?;
    let Some(Move::Member(category)) = resolved.moves().first() else {
        panic!("the first move should be `category`")
    };
    assert!(category.repeats());
    assert_eq!(category.max(), None);
    assert_eq!(category.min(), 0);
    assert_eq!(category.kind(), Kind::Complex("CodeableConcept"));
    Ok(())
}

#[test]
fn a_choice_alternative_answers_the_type_its_suffix_names()
-> Result<(), Box<dyn core::error::Error>> {
    for (expression, code) in [
        ("$resource.effectiveDateTime", "dateTime"),
        ("$resource.effective.ofType(dateTime)", "dateTime"),
        ("$resource.effectivePeriod", "Period"),
        ("$resource.valueQuantity", "Quantity"),
        ("$resource.valueString", "string"),
        ("$resource.component.valueString", "string"),
        ("$resource.component.valueBoolean", "boolean"),
        (
            "$resource.extension('http://example.org/fhir/StructureDefinition/probe').valueCodeableConcept",
            "CodeableConcept",
        ),
        (
            "$resource.extension('http://example.org/fhir/StructureDefinition/probe').valueDateTime",
            "dateTime",
        ),
    ] {
        let resolved = resolved("Observation", expression)?;
        assert_eq!(resolved.type_code(), Some(code), "{expression}");
    }
    Ok(())
}

#[test]
fn a_resolved_alternative_keeps_only_its_own_type_code() -> Result<(), Box<dyn core::error::Error>>
{
    let resolved = resolved("Observation", "$resource.effectiveDateTime")?;
    let Some(Move::Member(field)) = resolved.moves().last() else {
        panic!("a resolved alternative should be a member move")
    };
    assert_eq!(field.path(), "Observation.effective[x]");
    assert_eq!(field.key(), "effectiveDateTime");
    assert_eq!(field.types(), ["dateTime"]);
    Ok(())
}

#[test]
fn an_unresolved_choice_still_has_no_single_type() -> Result<(), Box<dyn core::error::Error>> {
    let resolved = resolved("Observation", "$resource.effective")?;
    assert_eq!(resolved.type_code(), None);
    Ok(())
}
