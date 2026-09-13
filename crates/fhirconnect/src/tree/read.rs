// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Reading a path over a FHIR JSON document.
//!
//! A read returns every value the expression matches, each with the structured
//! occurrence index of the repeating elements it passed, so the interpreter can
//! pair a FHIR occurrence with an openEHR one. The document model is the
//! lexical `Value` of `fhir-types`, so a decimal keeps the text it was written
//! with (<https://hl7.org/fhir/R4/datatypes.html#decimal>).
//!
//! `resolve()`, and a resource type asserted on a `Reference`, return a
//! [`Deferred`] outcome rather than a resource: the path model fetches
//! nothing, and the engine finishes the resolution.

use fhir_types::codec::Value;

use crate::tree::Occurrence;
use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::Table;
use crate::tree::element::resolve;
use crate::tree::error::ReadError;
use crate::tree::json_kind;
use crate::tree::path::FhirPath;
use crate::tree::path::Ordinal;
use crate::tree::path::Step;

/// A reference the engine resolves, with what remains of the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferred<'tree> {
    source: &'tree Value,
    expected: Option<String>,
    continuation: Vec<Step>,
}

impl<'tree> Deferred<'tree> {
    /// Returns the `Reference` element or uri the path reached.
    #[must_use]
    pub const fn source(&self) -> &'tree Value {
        self.source
    }

    /// Returns the resource type a type filter asserted, when one did.
    #[must_use]
    pub fn expected(&self) -> Option<&str> {
        self.expected.as_deref()
    }

    /// Returns the steps to apply once the reference is resolved.
    #[must_use]
    pub fn continuation(&self) -> &[Step] {
        &self.continuation
    }
}

/// What one match selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected<'tree> {
    /// A value in the document.
    Value(&'tree Value),
    /// A reference the engine resolves.
    Deferred(Deferred<'tree>),
}

/// One value a path matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match<'tree> {
    occurrence: Occurrence,
    selected: Selected<'tree>,
}

impl<'tree> Match<'tree> {
    /// Returns the index of each repeating element the match passed.
    #[must_use]
    pub const fn occurrence(&self) -> &Occurrence {
        &self.occurrence
    }

    /// Returns what the match selected.
    #[must_use]
    pub const fn selected(&self) -> &Selected<'tree> {
        &self.selected
    }

    /// Returns the value, `None` when the match is a deferred reference.
    #[must_use]
    pub const fn value(&self) -> Option<&'tree Value> {
        match self.selected {
            Selected::Value(value) => Some(value),
            Selected::Deferred(_) => None,
        }
    }
}

/// Reads `path` over `document`, returning every value it matches.
///
/// The resource type comes from the document's own `resourceType`
/// (<https://hl7.org/fhir/R4/json.html>). An element the document does not
/// carry matches nothing, which is an empty result rather than a refusal; a
/// document whose shape contradicts the element table is refused.
///
/// # Errors
///
/// Returns [`ReadError`] when the document names no resource type, when the
/// path does not resolve against the element table, when the document holds a
/// JSON kind the element's cardinality forbids, when a choice element carries
/// two alternatives, or when the path holds a `where()` predicate.
pub fn read<'tree, T: Table + ?Sized>(
    table: &T,
    document: &'tree Value,
    path: &FhirPath,
) -> Result<Vec<Match<'tree>>, ReadError> {
    let resource = document
        .get("resourceType")
        .and_then(Value::as_str)
        .ok_or(ReadError::NoResourceType)?;
    let resolved =
        resolve(table, resource, path).map_err(|source| ReadError::Resolve { source })?;
    let mut cursors = vec![Cursor {
        value: document,
        occurrence: Occurrence::default(),
    }];
    for step in resolved.moves() {
        cursors = advance(cursors, step)?;
    }
    let deferred = matches!(resolved.location(), Location::Deferred);
    Ok(cursors
        .into_iter()
        .map(|cursor| cursor.into_match(resolved.moves(), deferred))
        .collect())
}

/// One place in the document the walk has reached.
#[derive(Debug, Clone)]
struct Cursor<'tree> {
    value: &'tree Value,
    occurrence: Occurrence,
}

impl<'tree> Cursor<'tree> {
    /// Turns the cursor into the match it stands for.
    fn into_match(self, moves: &[Move], deferred: bool) -> Match<'tree> {
        let selected = match moves.last() {
            Some(Move::Resolve {
                expected,
                continuation,
            }) if deferred => Selected::Deferred(Deferred {
                source: self.value,
                expected: expected.clone(),
                continuation: continuation.clone(),
            }),
            _ => Selected::Value(self.value),
        };
        Match {
            occurrence: self.occurrence,
            selected,
        }
    }

    /// The same place one index deeper.
    fn indexed(&self, value: &'tree Value, index: usize) -> Self {
        let mut occurrence = self.occurrence.clone();
        occurrence.push(index);
        Self { value, occurrence }
    }
}

/// Takes one move over every cursor.
fn advance<'tree>(
    cursors: Vec<Cursor<'tree>>,
    step: &Move,
) -> Result<Vec<Cursor<'tree>>, ReadError> {
    match step {
        Move::Member(field) => {
            let mut next = Vec::new();
            for cursor in &cursors {
                member(
                    cursor,
                    field.path(),
                    field.key(),
                    field.repeats(),
                    &mut next,
                )?;
            }
            Ok(next)
        }
        Move::Choice { field, variants } => {
            let mut next = Vec::new();
            for cursor in &cursors {
                let Some(suffix) = alternative(cursor, field.path(), field.key(), variants)? else {
                    continue;
                };
                let key = format!("{}{suffix}", field.key());
                member(cursor, field.path(), &key, field.repeats(), &mut next)?;
            }
            Ok(next)
        }
        Move::Extension { field, url } => {
            let mut next = Vec::new();
            for cursor in &cursors {
                extension(cursor, field.path(), field.key(), url, &mut next)?;
            }
            Ok(next)
        }
        Move::Ordinal(Ordinal::First) => Ok(cursors.into_iter().take(1).collect()),
        Move::Ordinal(Ordinal::Last) => Ok(cursors.into_iter().next_back().into_iter().collect()),
        Move::Index(position) => Ok(cursors.into_iter().skip(*position).take(1).collect()),
        Move::Predicate(expression) => Err(ReadError::Predicate {
            expression: expression.clone(),
        }),
        Move::Resolve { .. } => Ok(cursors),
    }
}

/// Steps into one object member.
fn member<'tree>(
    cursor: &Cursor<'tree>,
    element: &str,
    key: &str,
    repeats: bool,
    next: &mut Vec<Cursor<'tree>>,
) -> Result<(), ReadError> {
    let object = cursor.value.as_object().ok_or_else(|| ReadError::Shape {
        element: String::from(element),
        expected: "an object",
        found: json_kind(cursor.value),
    })?;
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    if !repeats {
        if value.as_array().is_some() {
            return Err(ReadError::Shape {
                element: String::from(element),
                expected: "a single value",
                found: json_kind(value),
            });
        }
        next.push(Cursor {
            value,
            occurrence: cursor.occurrence.clone(),
        });
        return Ok(());
    }
    let items = value.as_array().ok_or_else(|| ReadError::Shape {
        element: String::from(element),
        expected: "an array",
        found: json_kind(value),
    })?;
    for (index, item) in items.iter().enumerate() {
        if item.is_null() {
            continue;
        }
        next.push(cursor.indexed(item, index));
    }
    Ok(())
}

/// The suffix of the alternative a document carries for a choice element.
fn alternative(
    cursor: &Cursor<'_>,
    element: &str,
    stem: &str,
    variants: &[(&'static str, fhir_types::xml::Kind)],
) -> Result<Option<&'static str>, ReadError> {
    let object = cursor.value.as_object().ok_or_else(|| ReadError::Shape {
        element: String::from(element),
        expected: "an object",
        found: json_kind(cursor.value),
    })?;
    let mut found: Option<&'static str> = None;
    for (suffix, _) in variants {
        let key = format!("{stem}{suffix}");
        if !object.contains_key(&key) {
            continue;
        }
        if let Some(first) = found {
            return Err(ReadError::AmbiguousChoice {
                element: String::from(element),
                first: format!("{stem}{first}"),
                second: key,
            });
        }
        found = Some(suffix);
    }
    Ok(found)
}

/// Steps into the extensions carrying one url.
fn extension<'tree>(
    cursor: &Cursor<'tree>,
    element: &str,
    key: &str,
    url: &str,
    next: &mut Vec<Cursor<'tree>>,
) -> Result<(), ReadError> {
    let object = cursor.value.as_object().ok_or_else(|| ReadError::Shape {
        element: String::from(element),
        expected: "an object",
        found: json_kind(cursor.value),
    })?;
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    let items = value.as_array().ok_or_else(|| ReadError::Shape {
        element: String::from(element),
        expected: "an array",
        found: json_kind(value),
    })?;
    let mut position = 0usize;
    for item in items {
        if item.get("url").and_then(Value::as_str) != Some(url) {
            continue;
        }
        next.push(cursor.indexed(item, position));
        position = position.saturating_add(1);
    }
    Ok(())
}
