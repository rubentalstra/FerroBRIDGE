// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Write then read is the identity on every element kind.

use core::error::Error;
use core::str::FromStr;

use fhir_types::codec::Number;
use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::Occurrence;
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::read::read;
use proptest::prelude::ProptestConfig;
use proptest::prelude::any;
use proptest::prelude::prop_assert;
use proptest::prelude::prop_assert_eq;
use proptest::proptest;
use proptest::test_runner::TestCaseError;

use crate::tree::depth;
use crate::tree::document;
use crate::tree::get;
use crate::tree::one;
use crate::tree::set;

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
