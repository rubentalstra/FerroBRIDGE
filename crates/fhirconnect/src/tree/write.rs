// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Writing a value at a path in a FHIR JSON document.
//!
//! The write creates what the element table's cardinality says to create: a
//! repeating element becomes an array the occurrence index extends by one at a
//! time, a scalar becomes the object member, a resolved choice becomes the
//! suffixed member, and a primitive's extension becomes the sibling named with
//! a leading underscore (<https://hl7.org/fhir/R4/json.html>).
//!
//! A write is all or nothing. It runs over a copy and replaces the document
//! only when every step succeeded, so a refusal leaves no half-built element
//! behind.

use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhir_types::schema::ValueKind;

use crate::tree::Occurrence;
use crate::tree::element::Field;
use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::Resolved;
use crate::tree::element::Table;
use crate::tree::element::resolve;
use crate::tree::error::WriteError;
use crate::tree::json_kind;
use crate::tree::path::FhirPath;
use crate::tree::path::Writability;

/// Writes `value` at `path` in `document`, at the occurrence `occurrence`
/// names.
///
/// One index of `occurrence` belongs to each repeating element the path steps
/// through, in path order, and an index one past the end appends. An
/// `extension(url)` step counts as a repeating element whose index runs over
/// the entries carrying that url.
///
/// # Errors
///
/// Returns [`WriteError`] when the expression is read-only, when the document
/// names no resource type, when the path does not resolve, when the path ends
/// on an unresolved choice or a deferred reference, when a repeating element
/// has no index or an index that would leave a gap, when the document holds a
/// JSON kind the walk cannot step through, or when the value is of a kind the
/// element does not admit.
pub fn write<T: Table + ?Sized>(
    table: &T,
    document: &mut Value,
    path: &FhirPath,
    occurrence: &Occurrence,
    value: Value,
) -> Result<(), WriteError> {
    if let Writability::ReadOnly { step, reason } = path.writability() {
        return Err(WriteError::ReadOnly {
            expression: String::from(path.as_str()),
            step: step.clone(),
            reason,
        });
    }
    let resource = document
        .get("resourceType")
        .and_then(Value::as_str)
        .ok_or(WriteError::NoResourceType)?;
    let resolved =
        resolve(table, resource, path).map_err(|source| WriteError::Resolve { source })?;
    admit(&resolved, path, &value)?;
    let mut draft = document.clone();
    let mut node = &mut draft;
    let mut spent = 0usize;
    for step in resolved.moves() {
        node = descend(node, step, occurrence, &mut spent)?;
    }
    if let Some(extra) = occurrence.indices().get(spent) {
        return Err(WriteError::NotRepeating {
            element: String::from(resolved.leaf()),
            index: *extra,
        });
    }
    *node = value;
    *document = draft;
    Ok(())
}

/// Refuses a value the element does not admit, before anything is written.
fn admit(resolved: &Resolved, path: &FhirPath, value: &Value) -> Result<(), WriteError> {
    let element = String::from(resolved.leaf());
    let expected = match resolved.location() {
        Location::Choice(choice) => {
            return Err(WriteError::UnresolvedChoice {
                element: choice.clone(),
            });
        }
        Location::Deferred => {
            return Err(WriteError::ReferenceTarget {
                expression: String::from(path.as_str()),
            });
        }
        Location::Complex(_) | Location::PrimitiveElement | Location::Resource => "an object",
        Location::Attribute | Location::Primitive(ValueKind::Text) => "a string",
        Location::Primitive(ValueKind::Boolean) => "a boolean",
        Location::Primitive(ValueKind::Integer) => "a whole number",
        Location::Primitive(ValueKind::Decimal) => "a number",
    };
    if value.is_null() {
        return Err(WriteError::NullValue { element });
    }
    let admitted = match (resolved.location(), value) {
        (
            Location::Complex(_) | Location::PrimitiveElement | Location::Resource,
            Value::Object(_),
        )
        | (Location::Attribute | Location::Primitive(ValueKind::Text), Value::String(_))
        | (Location::Primitive(ValueKind::Boolean), Value::Bool(_))
        | (Location::Primitive(ValueKind::Decimal), Value::Number(_)) => true,
        (Location::Primitive(ValueKind::Integer), Value::Number(number)) => {
            number.as_i64().is_some()
        }
        _ => false,
    };
    if admitted {
        return Ok(());
    }
    Err(WriteError::TypeNotAdmitted {
        element,
        expected,
        found: json_kind(value),
    })
}

/// Takes one move, creating what the element table says the document holds.
fn descend<'draft>(
    node: &'draft mut Value,
    step: &Move,
    occurrence: &Occurrence,
    spent: &mut usize,
) -> Result<&'draft mut Value, WriteError> {
    match step {
        Move::Member(field) => {
            let object = object_mut(node, field.path())?;
            if field.repeats() {
                let index = take(occurrence, spent, field.path())?;
                return slot(object, field.key(), field.path(), index);
            }
            scalar(object, field)
        }
        Move::Extension { field, url } => {
            let object = object_mut(node, field.path())?;
            let index = take(occurrence, spent, field.path())?;
            entry(object, field, url, index)
        }
        Move::Choice { field, .. } => Err(WriteError::UnresolvedChoice {
            element: String::from(field.path()),
        }),
        Move::Resolve { .. } => Err(WriteError::ReferenceTarget {
            expression: String::from("resolve()"),
        }),
        Move::Ordinal(ordinal) => Err(WriteError::ReadOnly {
            expression: ordinal.to_string(),
            step: ordinal.to_string(),
            reason: "picks one of the values a document already holds",
        }),
        Move::Index(index) => Err(WriteError::ReadOnly {
            expression: format!("[{index}]"),
            step: format!("[{index}]"),
            reason: "picks one position of the values a document already holds",
        }),
        Move::Predicate(expression) => Err(WriteError::ReadOnly {
            expression: format!("where({expression})"),
            step: format!("where({expression})"),
            reason: "is a predicate over the values a document already holds",
        }),
    }
}

/// The next occurrence index, or the refusal for a repeating element with
/// none.
fn take(occurrence: &Occurrence, spent: &mut usize, element: &str) -> Result<usize, WriteError> {
    let index =
        occurrence
            .indices()
            .get(*spent)
            .copied()
            .ok_or_else(|| WriteError::MissingOccurrence {
                element: String::from(element),
            })?;
    *spent = spent.saturating_add(1);
    Ok(index)
}

/// The object a step writes into, created when the member is absent.
fn object_mut<'draft>(
    node: &'draft mut Value,
    element: &str,
) -> Result<&'draft mut Object, WriteError> {
    if node.is_null() {
        *node = Value::Object(Object::new());
    }
    match node {
        Value::Object(object) => Ok(object),
        other => Err(WriteError::Shape {
            element: String::from(element),
            expected: "an object",
            found: json_kind(other),
        }),
    }
}

/// The member of a scalar element, created when it is absent.
fn scalar<'draft>(
    object: &'draft mut Object,
    field: &Field,
) -> Result<&'draft mut Value, WriteError> {
    let slot = object
        .entry(String::from(field.key()))
        .or_insert(Value::Null);
    if slot.as_array().is_some() {
        return Err(WriteError::Shape {
            element: String::from(field.path()),
            expected: "a single value",
            found: "an array",
        });
    }
    Ok(slot)
}

/// The `index`-th occurrence of a repeating element, appended when it is the
/// next one.
fn slot<'draft>(
    object: &'draft mut Object,
    key: &str,
    element: &str,
    index: usize,
) -> Result<&'draft mut Value, WriteError> {
    let member = object
        .entry(String::from(key))
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(items) = member else {
        return Err(WriteError::Shape {
            element: String::from(element),
            expected: "an array",
            found: json_kind(member),
        });
    };
    if index > items.len() {
        return Err(WriteError::OccurrenceGap {
            element: String::from(element),
            index,
            length: items.len(),
        });
    }
    if index == items.len() {
        items.push(Value::Null);
    }
    items
        .get_mut(index)
        .ok_or_else(|| WriteError::OccurrenceGap {
            element: String::from(element),
            index,
            length: 0,
        })
}

/// The `index`-th extension carrying `url`, appended when it is the next one.
fn entry<'draft>(
    object: &'draft mut Object,
    field: &Field,
    url: &str,
    index: usize,
) -> Result<&'draft mut Value, WriteError> {
    let element = String::from(field.path());
    let member = object
        .entry(String::from(field.key()))
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(items) = member else {
        return Err(WriteError::Shape {
            element,
            expected: "an array",
            found: json_kind(member),
        });
    };
    let positions: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.get("url").and_then(Value::as_str) == Some(url))
        .map(|(position, _)| position)
        .collect();
    if let Some(position) = positions.get(index) {
        let position = *position;
        let length = positions.len();
        return items.get_mut(position).ok_or(WriteError::OccurrenceGap {
            element,
            index,
            length,
        });
    }
    if index != positions.len() {
        return Err(WriteError::OccurrenceGap {
            element,
            index,
            length: positions.len(),
        });
    }
    let mut created = Object::new();
    let _url = created.insert(String::from("url"), Value::String(String::from(url)));
    items.push(Value::Object(created));
    items.last_mut().ok_or(WriteError::OccurrenceGap {
        element,
        index,
        length: positions.len(),
    })
}
