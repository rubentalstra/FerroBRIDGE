// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every refusal names the element the table holds.

use core::error::Error;

use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::Occurrence;
use fhirconnect::tree::element::resolve;
use fhirconnect::tree::error::ReadError;
use fhirconnect::tree::error::ResolveError;
use fhirconnect::tree::error::WriteError;
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::read::read;
use fhirconnect::tree::write::write;

use crate::tree::document;
use crate::tree::set;

#[test]
fn an_unknown_element_names_the_type_that_defines_none() -> Result<(), Box<dyn Error>> {
    let document = document("Condition");
    let path: FhirPath = "$resource.onsetDate".parse()?;
    let refused = read(&SCHEMAS, &document, &path);
    let Err(ReadError::Resolve {
        source: ResolveError::UnknownElement { owner, name },
    }) = refused
    else {
        panic!("an unknown element should have been refused")
    };
    assert_eq!(owner, "Condition");
    assert_eq!(name, "onsetDate");
    Ok(())
}

#[test]
fn a_type_specifier_that_names_no_fhir_type_is_refused() -> Result<(), Box<dyn Error>> {
    let path: FhirPath = "$resource.onset.ofType(DateTimeType)".parse()?;
    let Err(ResolveError::ChoiceType {
        element,
        requested,
        admitted,
    }) = resolve(&SCHEMAS, "Condition", &path)
    else {
        panic!("a type specifier naming no FHIR type should have been refused")
    };
    assert_eq!(element, "Condition.onset[x]");
    assert_eq!(requested, "DateTimeType");
    assert_eq!(admitted, "DateTime, Age, Period, Range, String");
    Ok(())
}

#[test]
fn a_choice_type_the_element_does_not_admit_names_the_element() -> Result<(), Box<dyn Error>> {
    let mut document = document("Condition");
    let refused = set(
        &mut document,
        "$resource.onset.ofType(Quantity)",
        &[],
        Value::String(String::from("2026-01-01")),
    );
    let message = refused.err().map(|error| error.to_string());
    assert_eq!(
        message.as_deref(),
        Some("the path does not resolve"),
        "the write should have been refused by the element table"
    );
    let path: FhirPath = "$resource.onset.ofType(Quantity)".parse()?;
    let Err(ResolveError::ChoiceType {
        element, admitted, ..
    }) = resolve(&SCHEMAS, "Condition", &path)
    else {
        panic!("a wrong choice type should have been refused")
    };
    assert_eq!(element, "Condition.onset[x]");
    assert_eq!(admitted, "DateTime, Age, Period, Range, String");
    Ok(())
}

#[test]
fn a_write_into_a_repeating_element_without_an_index_names_the_element()
-> Result<(), Box<dyn Error>> {
    let empty = document("Condition");
    let mut document = document("Condition");
    let path: FhirPath = "$resource.category.coding.code".parse()?;
    let refused = write(
        &SCHEMAS,
        &mut document,
        &path,
        &Occurrence::default(),
        Value::String(String::from("synthetic-code")),
    );
    assert!(matches!(
        refused,
        Err(WriteError::MissingOccurrence { ref element }) if element == "Condition.category"
    ));
    assert_eq!(document, empty, "nothing should be written");
    Ok(())
}

#[test]
fn an_occurrence_index_on_a_scalar_element_names_the_element() -> Result<(), Box<dyn Error>> {
    let mut document = document("Condition");
    let path: FhirPath = "$resource.recordedDate".parse()?;
    let refused = write(
        &SCHEMAS,
        &mut document,
        &path,
        &Occurrence::new([2]),
        Value::String(String::from("2026-01-01")),
    );
    assert!(matches!(
        refused,
        Err(WriteError::NotRepeating { ref element, index: 2 })
            if element == "Condition.recordedDate"
    ));
    Ok(())
}

#[test]
fn an_index_past_the_end_of_an_array_names_the_element() -> Result<(), Box<dyn Error>> {
    let mut document = document("Condition");
    let path: FhirPath = "$resource.category".parse()?;
    let refused = write(
        &SCHEMAS,
        &mut document,
        &path,
        &Occurrence::new([3]),
        Value::Object(Object::new()),
    );
    assert!(matches!(
        refused,
        Err(WriteError::OccurrenceGap { ref element, index: 3, length: 0 })
            if element == "Condition.category"
    ));
    Ok(())
}

#[test]
fn a_value_of_a_kind_the_element_does_not_admit_names_the_element() -> Result<(), Box<dyn Error>> {
    let mut document = document("Patient");
    let path: FhirPath = "$resource.active".parse()?;
    let refused = write(
        &SCHEMAS,
        &mut document,
        &path,
        &Occurrence::default(),
        Value::String(String::from("true")),
    );
    assert!(matches!(
        refused,
        Err(WriteError::TypeNotAdmitted { ref element, expected: "a boolean", found: "a string" })
            if element == "Patient.active"
    ));
    Ok(())
}

#[test]
fn a_write_through_an_unresolved_choice_names_the_element() -> Result<(), Box<dyn Error>> {
    let mut document = document("Condition");
    let path: FhirPath = "$resource.onset".parse()?;
    let refused = write(
        &SCHEMAS,
        &mut document,
        &path,
        &Occurrence::default(),
        Value::String(String::from("2026-01-01")),
    );
    assert!(matches!(
        refused,
        Err(WriteError::UnresolvedChoice { ref element }) if element == "Condition.onset[x]"
    ));
    Ok(())
}

#[test]
fn a_write_side_that_filters_is_refused_with_the_expression_named() -> Result<(), Box<dyn Error>> {
    for (expression, step) in [
        (
            "$resource.category.where(coding.code = 'synthetic').text",
            "where(coding.code = 'synthetic')",
        ),
        ("$resource.category.first().text", "first()"),
        ("$resource.category[1].text", "[1]"),
    ] {
        let mut document = document("Condition");
        let path: FhirPath = expression.parse()?;
        let refused = write(
            &SCHEMAS,
            &mut document,
            &path,
            &Occurrence::default(),
            Value::String(String::from("synthetic")),
        );
        let Err(WriteError::ReadOnly {
            expression: named,
            step: offending,
            ..
        }) = refused
        else {
            panic!("`{expression}` should have been refused as read-only")
        };
        assert_eq!(named, expression);
        assert_eq!(offending, step);
    }
    Ok(())
}

#[test]
fn a_read_only_refusal_renders_the_expression_and_the_offending_step() -> Result<(), Box<dyn Error>>
{
    let mut document = document("Condition");
    let path: FhirPath = "$resource.category.first().text".parse()?;
    let refused = write(
        &SCHEMAS,
        &mut document,
        &path,
        &Occurrence::default(),
        Value::String(String::from("synthetic")),
    );
    let message = refused
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    insta::assert_snapshot!(
        message,
        @"`$resource.category.first().text` is read-only: `first()` picks one of the values a document already holds"
    );
    Ok(())
}

#[test]
fn a_wrong_choice_type_renders_the_element_and_its_alternatives() -> Result<(), Box<dyn Error>> {
    let path: FhirPath = "$resource.onset.ofType(Quantity)".parse()?;
    let refused = resolve(&SCHEMAS, "Condition", &path);
    let message = refused
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    insta::assert_snapshot!(
        message,
        @"`Condition.onset[x]` admits DateTime, Age, Period, Range, String, not `Quantity`"
    );
    Ok(())
}
