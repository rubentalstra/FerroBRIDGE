// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The diagnosis chain's paths and the round trips through the strict codec.

use core::error::Error;

use fhir_types::codec::Json;
use fhir_types::codec::Object;
use fhir_types::codec::Path;
use fhir_types::codec::Value;
use fhir_types::r4::condition::Condition;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::error::ReadError;
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::read::Match;
use fhirconnect::tree::read::Selected;
use fhirconnect::tree::read::read;

use crate::tree::ASSERTED_DATE;
use crate::tree::document;
use crate::tree::get;
use crate::tree::one;
use crate::tree::set;

/// A synthetic `Condition` carrying every shape the diagnose chain reads.
fn diagnose_condition() -> Result<Value, Box<dyn Error>> {
    let mut document = document("Condition");
    set(
        &mut document,
        "$resource.id",
        &[],
        Value::String(String::from("synthetic-condition-1")),
    )?;
    set(
        &mut document,
        "$resource.subject.reference",
        &[],
        Value::String(String::from("Patient/synthetic-subject-1")),
    )?;
    set(
        &mut document,
        "$resource.code.text",
        &[],
        Value::String(String::from("Synthetic finding for the test")),
    )?;
    set(
        &mut document,
        "$resource.code.coding.code",
        &[0],
        Value::String(String::from("SYN-1")),
    )?;
    set(
        &mut document,
        "$resource.code.coding.system",
        &[0],
        Value::String(String::from("http://example.invalid/synthetic-codes")),
    )?;
    set(
        &mut document,
        "$resource.onset.ofType(Period).start",
        &[],
        Value::String(String::from("2026-01-02T03:04:05+01:00")),
    )?;
    set(
        &mut document,
        "$resource.encounter.identifier.value",
        &[],
        Value::String(String::from("synthetic-encounter-1")),
    )?;
    set(
        &mut document,
        "$resource.recorder.reference",
        &[],
        Value::String(String::from("Practitioner/synthetic-recorder-1")),
    )?;
    let asserted = format!("$resource.extension('{ASSERTED_DATE}').value.ofType(DateTime)");
    set(
        &mut document,
        &asserted,
        &[0],
        Value::String(String::from("2026-01-01")),
    )?;
    Ok(document)
}

#[test]
fn the_diagnose_chain_reads_the_choice_through_both_spellings() -> Result<(), Box<dyn Error>> {
    let document = diagnose_condition()?;
    let anchor: FhirPath = "$resource".parse()?;
    let period: FhirPath = "onset.ofType(Period)"
        .parse::<FhirPath>()?
        .anchored(&anchor)?;
    assert_eq!(period.as_str(), "$resource.onset.ofType(Period)");
    assert_eq!(get(&document, period.as_str())?.len(), 1);
    assert!(
        one(&document, "onset.ofType(Period).start").is_err(),
        "a relative expression is refused until it is bound to an anchor"
    );
    let start: FhirPath = "$resource.onset.ofType(Period).start".parse()?;
    assert_eq!(
        read(&SCHEMAS, &document, &start)?
            .first()
            .and_then(Match::value)
            .and_then(Value::as_str),
        Some("2026-01-02T03:04:05+01:00")
    );
    let as_datetime: FhirPath = "$resource.onset.as(DateTime)".parse()?;
    assert!(
        read(&SCHEMAS, &document, &as_datetime)?.is_empty(),
        "the document carries the Period alternative, so the DateTime one matches nothing"
    );
    Ok(())
}

#[test]
fn the_diagnose_chain_reads_the_extension_by_url() -> Result<(), Box<dyn Error>> {
    let document = diagnose_condition()?;
    let expression = format!("$resource.extension('{ASSERTED_DATE}').value.as(DateTime)");
    assert_eq!(
        one(&document, &expression)?.and_then(Value::as_str),
        Some("2026-01-01")
    );
    let other = format!("$resource.extension('{ASSERTED_DATE}x').value.as(DateTime)");
    assert!(
        get(&document, &other)?.is_empty(),
        "another url matches nothing"
    );
    Ok(())
}

#[test]
fn the_diagnose_chain_reads_a_reference_as_a_deferred_outcome() -> Result<(), Box<dyn Error>> {
    let document = diagnose_condition()?;
    let path: FhirPath = "$resource.recorder.resolve().as(Practitioner).name.family".parse()?;
    let matched = read(&SCHEMAS, &document, &path)?;
    assert_eq!(matched.len(), 1);
    let Some(Selected::Deferred(deferred)) = matched.first().map(Match::selected) else {
        panic!("`resolve()` should have deferred")
    };
    assert_eq!(deferred.expected(), Some("Practitioner"));
    assert_eq!(deferred.continuation().len(), 2);
    assert_eq!(
        deferred.source().get("reference").and_then(Value::as_str),
        Some("Practitioner/synthetic-recorder-1"),
        "the deferred outcome carries the reference, never a fetched resource"
    );
    Ok(())
}

#[test]
fn the_composition_mapping_reads_the_encounter_identifier() -> Result<(), Box<dyn Error>> {
    let document = diagnose_condition()?;
    assert_eq!(
        one(
            &document,
            "$resource.encounter.ofType(Reference).identifier.value"
        )?
        .and_then(Value::as_str),
        Some("synthetic-encounter-1")
    );
    Ok(())
}

#[test]
fn a_tree_written_through_the_api_round_trips_through_the_strict_codec()
-> Result<(), Box<dyn Error>> {
    let document = diagnose_condition()?;
    let object = document.as_object().ok_or("the document is an object")?;
    let decoded = Condition::from_json(object, &mut Path::root("Condition"))?;
    let encoded = decoded.to_json()?;
    assert_eq!(
        &encoded, object,
        "the strict codec round trip changed the tree"
    );
    Ok(())
}

#[test]
fn a_primitive_extension_round_trips_through_the_underscore_sibling() -> Result<(), Box<dyn Error>>
{
    let mut document = document("Condition");
    set(
        &mut document,
        "$resource.subject.reference",
        &[],
        Value::String(String::from("Patient/synthetic-subject-1")),
    )?;
    set(
        &mut document,
        "$resource.onset.ofType(Period).start",
        &[],
        Value::String(String::from("2026-02-03")),
    )?;
    let expression =
        format!("$resource.onset.ofType(Period).start.extension('{ASSERTED_DATE}').value.as(Code)");
    set(
        &mut document,
        &expression,
        &[0],
        Value::String(String::from("synthetic-qualifier")),
    )?;
    let period = document
        .get("onsetPeriod")
        .and_then(Value::as_object)
        .ok_or("the period is an object")?;
    assert!(
        period.contains_key("_start"),
        "the extension of a primitive lives in the `_start` sibling"
    );
    assert_eq!(
        one(&document, &expression)?.and_then(Value::as_str),
        Some("synthetic-qualifier")
    );
    let object = document.as_object().ok_or("the document is an object")?;
    let decoded = Condition::from_json(object, &mut Path::root("Condition"))?;
    assert_eq!(&decoded.to_json()?, object);
    Ok(())
}

#[test]
fn a_document_with_two_alternatives_of_one_choice_is_refused() -> Result<(), Box<dyn Error>> {
    let mut document = document("Condition");
    set(
        &mut document,
        "$resource.onset.ofType(DateTime)",
        &[],
        Value::String(String::from("2026-01-01")),
    )?;
    set(
        &mut document,
        "$resource.onset.ofType(Period).start",
        &[],
        Value::String(String::from("2026-01-01")),
    )?;
    let path: FhirPath = "$resource.onset".parse()?;
    assert!(matches!(
        read(&SCHEMAS, &document, &path),
        Err(ReadError::AmbiguousChoice { ref element, .. }) if element == "Condition.onset[x]"
    ));
    Ok(())
}

#[test]
fn a_predicate_is_parsed_and_refused_at_evaluation() -> Result<(), Box<dyn Error>> {
    let document = diagnose_condition()?;
    let path: FhirPath = "$resource.code.coding.where(system = 'x').code".parse()?;
    assert!(matches!(
        read(&SCHEMAS, &document, &path),
        Err(ReadError::Predicate { ref expression }) if expression == "system = 'x'"
    ));
    Ok(())
}

#[test]
fn a_document_with_no_resource_type_is_refused() -> Result<(), Box<dyn Error>> {
    let document = Value::Object(Object::new());
    let path: FhirPath = "$resource.id".parse()?;
    assert!(matches!(
        read(&SCHEMAS, &document, &path),
        Err(ReadError::NoResourceType)
    ));
    Ok(())
}
