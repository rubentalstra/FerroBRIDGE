// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The tree module against the R4 element table.
//!
//! Read and write go through the public API over synthetic documents built in
//! the test. Every fixture value is invented here; no clinical content of any
//! kind appears in this file.

use core::error::Error;
use core::str::FromStr;

use fhir_types::codec::Json;
use fhir_types::codec::Number;
use fhir_types::codec::Object;
use fhir_types::codec::Path;
use fhir_types::codec::Value;
use fhir_types::r4::condition::Condition;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::Occurrence;
use fhirconnect::tree::element::Move;
use fhirconnect::tree::element::resolve;
use fhirconnect::tree::error::ReadError;
use fhirconnect::tree::error::ResolveError;
use fhirconnect::tree::error::WriteError;
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::read::Match;
use fhirconnect::tree::read::Selected;
use fhirconnect::tree::read::read;
use fhirconnect::tree::write::write;
use proptest::prelude::ProptestConfig;
use proptest::prelude::any;
use proptest::prelude::prop_assert;
use proptest::prelude::prop_assert_eq;
use proptest::proptest;
use proptest::test_runner::TestCaseError;

/// The url of the extension the diagnose chain selects.
///
/// It is the one the published FHIRconnect mapping library names for an
/// asserted date, and FHIR identifies an extension by its url
/// (<https://hl7.org/fhir/R4/extensibility.html>).
const ASSERTED_DATE: &str = "http://hl7.org/fhir/StructureDefinition/condition-assertedDate";

/// An empty document of one resource type.
fn document(resource: &str) -> Value {
    let mut object = Object::new();
    let _previous = object.insert(
        String::from("resourceType"),
        Value::String(String::from(resource)),
    );
    Value::Object(object)
}

/// Writes `value` at `expression`, at the occurrence `occurrence` names.
fn set(
    document: &mut Value,
    expression: &str,
    occurrence: &[usize],
    value: Value,
) -> Result<(), Box<dyn Error>> {
    let path: FhirPath = expression.parse()?;
    write(
        &SCHEMAS,
        document,
        &path,
        &Occurrence::new(occurrence.to_vec()),
        value,
    )?;
    Ok(())
}

/// Reads every value `expression` matches.
fn get<'tree>(
    document: &'tree Value,
    expression: &str,
) -> Result<Vec<&'tree Value>, Box<dyn Error>> {
    let path: FhirPath = expression.parse()?;
    Ok(read(&SCHEMAS, document, &path)?
        .iter()
        .filter_map(Match::value)
        .collect())
}

/// The one value `expression` matches, `None` when it matches nothing.
fn one<'tree>(
    document: &'tree Value,
    expression: &str,
) -> Result<Option<&'tree Value>, Box<dyn Error>> {
    let found = get(document, expression)?;
    Ok(found.first().copied())
}

/// How many repeating elements a path steps through.
fn depth(resource: &str, expression: &str) -> Result<usize, Box<dyn Error>> {
    let path: FhirPath = expression.parse()?;
    let resolved = resolve(&SCHEMAS, resource, &path)?;
    Ok(resolved
        .moves()
        .iter()
        .filter(|step| match step {
            Move::Member(field) => field.repeats(),
            Move::Extension { .. } => true,
            Move::Choice { .. }
            | Move::Ordinal(_)
            | Move::Index(_)
            | Move::Predicate(_)
            | Move::Resolve { .. } => false,
        })
        .count())
}

/// The kind of value an element admits, so the case table can build one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// A JSON string.
    Text,
    /// A JSON boolean.
    Boolean,
    /// A JSON whole number.
    Integer,
    /// A JSON decimal, kept in its lexical form.
    Decimal,
    /// A JSON object.
    Complex,
}

/// One element kind of one resource, with the expression that names it.
struct Case {
    resource: &'static str,
    expression: &'static str,
    shape: Shape,
}

/// The bounded subset of the R4 element table the properties run over.
///
/// It covers each element kind the JSON representation distinguishes
/// (<https://hl7.org/fhir/R4/json.html>): a primitive, a complex element, both
/// halves of a choice, a repeating element, a repeating primitive, and the
/// extension of a primitive, which lives in the `_start` sibling.
const CASES: &[Case] = &[
    Case {
        resource: "Condition",
        expression: "$resource.recordedDate",
        shape: Shape::Text,
    },
    Case {
        resource: "Condition",
        expression: "$resource.code",
        shape: Shape::Complex,
    },
    Case {
        resource: "Condition",
        expression: "$resource.onset.ofType(DateTime)",
        shape: Shape::Text,
    },
    Case {
        resource: "Condition",
        expression: "$resource.onset.ofType(Period)",
        shape: Shape::Complex,
    },
    Case {
        resource: "Condition",
        expression: "$resource.category",
        shape: Shape::Complex,
    },
    Case {
        resource: "Condition",
        expression: "$resource.category.coding.code",
        shape: Shape::Text,
    },
    Case {
        resource: "Condition",
        expression: "$resource.onset.ofType(Period).start.extension.value.ofType(DateTime)",
        shape: Shape::Text,
    },
    Case {
        resource: "Condition",
        expression: "$resource.extension.value.ofType(Integer)",
        shape: Shape::Integer,
    },
    Case {
        resource: "Observation",
        expression: "$resource.value.ofType(Quantity).value",
        shape: Shape::Decimal,
    },
    Case {
        resource: "Observation",
        expression: "$resource.component.value.ofType(Quantity).unit",
        shape: Shape::Text,
    },
    Case {
        resource: "Patient",
        expression: "$resource.active",
        shape: Shape::Boolean,
    },
    Case {
        resource: "Patient",
        expression: "$resource.name.given",
        shape: Shape::Text,
    },
    Case {
        resource: "Patient",
        expression: "$resource.deceased.ofType(Boolean)",
        shape: Shape::Boolean,
    },
    Case {
        resource: "Encounter",
        expression: "$resource.period.start",
        shape: Shape::Text,
    },
    Case {
        resource: "Encounter",
        expression: "$resource.identifier.value",
        shape: Shape::Text,
    },
];

/// The two elements every case leaves alone, so a write can be shown local.
const UNTOUCHED: [(&str, &str); 2] = [
    ("$resource.id", "synthetic-record-1"),
    ("$resource.language", "en"),
];

/// Turns any failure into a proptest failure carrying its text.
fn fail(error: impl core::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

/// A value of the shape a case's element admits.
fn value(shape: Shape, text: &str, flag: bool, whole: i32, decimal: &str) -> Result<Value, String> {
    Ok(match shape {
        Shape::Text => Value::String(String::from(text)),
        Shape::Boolean => Value::Bool(flag),
        Shape::Integer => Value::from(whole),
        Shape::Decimal => {
            Value::Number(Number::from_str(decimal).map_err(|error| error.to_string())?)
        }
        Shape::Complex => {
            let mut object = Object::new();
            let _previous = object.insert(
                String::from("id"),
                Value::String(String::from("synthetic-element-1")),
            );
            Value::Object(object)
        }
    })
}

/// A document of the case's resource with the two untouched elements set.
fn seeded(resource: &str) -> Result<Value, Box<dyn Error>> {
    let mut document = document(resource);
    for (expression, text) in UNTOUCHED {
        set(
            &mut document,
            expression,
            &[],
            Value::String(String::from(text)),
        )?;
    }
    Ok(document)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Write then read at one expression returns the value that was written.
    #[test]
    fn write_then_read_is_the_identity_on_every_element_kind(
        which in 0usize..CASES.len(),
        text in "[A-Za-z0-9-]{1,12}",
        flag in any::<bool>(),
        whole in -1000i32..1000,
        decimal in "(0|[1-9][0-9]{0,2})\\.[0-9]{1,4}",
    ) {
        let case = CASES.get(which).ok_or_else(|| fail("no case"))?;
        let written = value(case.shape, &text, flag, whole, &decimal).map_err(fail)?;
        let mut document = seeded(case.resource).map_err(fail)?;
        let indices = vec![0usize; depth(case.resource, case.expression).map_err(fail)?];
        set(&mut document, case.expression, &indices, written.clone()).map_err(fail)?;
        let found = get(&document, case.expression).map_err(fail)?;
        prop_assert_eq!(found.len(), 1, "{} matched {:?}", case.expression, found);
        prop_assert_eq!(found.first().copied(), Some(&written));
    }

    /// A write changes the element it names and nothing else.
    #[test]
    fn a_write_leaves_every_other_element_untouched(
        which in 0usize..CASES.len(),
        text in "[A-Za-z0-9-]{1,12}",
        flag in any::<bool>(),
        whole in -1000i32..1000,
        decimal in "(0|[1-9][0-9]{0,2})\\.[0-9]{1,4}",
    ) {
        let case = CASES.get(which).ok_or_else(|| fail("no case"))?;
        let written = value(case.shape, &text, flag, whole, &decimal).map_err(fail)?;
        let mut document = seeded(case.resource).map_err(fail)?;
        let indices = vec![0usize; depth(case.resource, case.expression).map_err(fail)?];
        set(&mut document, case.expression, &indices, written).map_err(fail)?;
        for (expression, expected) in UNTOUCHED {
            let found = one(&document, expression).map_err(fail)?;
            prop_assert_eq!(found, Some(&Value::String(String::from(expected))));
        }
    }

    /// Appending occurrences reads them back in the order they were written.
    #[test]
    fn appended_occurrences_read_back_in_order(count in 1usize..6) {
        let mut document = seeded("Condition").map_err(fail)?;
        let expression = "$resource.category.coding.code";
        for index in 0..count {
            let code = format!("synthetic-code-{index}");
            set(&mut document, expression, &[index, 0], Value::String(code)).map_err(fail)?;
        }
        let found = get(&document, expression).map_err(fail)?;
        prop_assert_eq!(found.len(), count);
        for (index, value) in found.iter().enumerate() {
            let expected = Value::String(format!("synthetic-code-{index}"));
            prop_assert_eq!(*value, &expected);
        }
        let path: FhirPath = expression.parse().map_err(fail)?;
        let matched = read(&SCHEMAS, &document, &path).map_err(fail)?;
        for (index, found) in matched.iter().enumerate() {
            prop_assert_eq!(found.occurrence(), &Occurrence::new(vec![index, 0]));
        }
        prop_assert!(!matched.is_empty());
    }
}

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
