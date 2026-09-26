// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Conditions, evaluated on the input side only.
//!
//! "Conditions are always applied on the input data. They do not filter on the
//! data that is currently mapped. When we map a FHIR resource into openEHR,
//! only the fhirConditions are processed and in case of openEHR to FHIR it's
//! the other way around"
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`). That
//! sentence is the whole of the direction rule here: [`runs`] answers whether
//! a compiled condition belongs to the direction being run, and a condition
//! that does not is never evaluated.
//!
//! A condition whose `targetRoot` is the `with` input path filters that path's
//! occurrences; one that points elsewhere is "handled as simple true/false".
//! The five operators carry the connectives the page's table fixes: `one of`
//! is an OR over the criteria, `not of`, `empty` and `not empty` are ANDs, and
//! `type` is an OR over the named types.

use fhir_types::codec::Value;

use crate::model::ast::keyword::ConditionOperator;
use crate::model::ast::keyword::Direction;
use crate::resolve::program::condition::Condition;
use crate::tree::Occurrence;
use crate::tree::element::Location;
use crate::tree::element::Table;
use crate::tree::error::ReadError;
use crate::tree::path::FhirPath;
use crate::tree::read::Match;
use crate::tree::read::Selected;
use crate::tree::read::read;

/// Returns whether `direction` evaluates `condition`.
///
/// A `fhirCondition` runs when FHIR is the input and an `openehrCondition`
/// when openEHR is, so the two never both apply to one run.
#[must_use]
pub const fn runs(condition: &Condition, direction: Direction) -> bool {
    matches!(
        (condition.direction(), direction),
        (Direction::FhirToOpenehr, Direction::FhirToOpenehr)
            | (Direction::OpenehrToFhir, Direction::OpenehrToFhir)
    )
}

/// What a condition decided about the input it guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The condition gates the mapping as a whole.
    Gate(bool),
    /// The condition filters the occurrences of the input path.
    Filter(Vec<Occurrence>),
}

impl Verdict {
    /// Returns whether the mapping runs at all.
    #[must_use]
    pub fn admits_any(&self) -> bool {
        match *self {
            Self::Gate(passed) => passed,
            Self::Filter(ref occurrences) => !occurrences.is_empty(),
        }
    }
}

/// Why a condition could not be decided.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConditionError {
    /// The condition's path could not be read over the document.
    #[error("the condition on {target} could not be read")]
    Read {
        /// The expression the condition names.
        target: String,
        /// The refusal the path model returned.
        #[source]
        source: ReadError,
    },
    /// The condition names the openEHR side while FHIR is the input.
    #[error("the condition on the openEHR side cannot be read over a FHIR document")]
    WrongSide,
    /// The compiled condition has no target attribute for its operator.
    #[error("the {operator} operator needs a target attribute, and the condition names none")]
    NoAttribute {
        /// The operator that needs one.
        operator: ConditionOperator,
    },
}

/// Evaluates one compiled `fhirCondition` over `document`.
///
/// `attached` says whether the condition's `targetRoot` is the `with` input
/// path. An attached condition returns the occurrences of that path the
/// condition admits; an unattached one returns a plain gate.
///
/// # Errors
///
/// Returns [`ConditionError::WrongSide`] for a condition compiled against the
/// openEHR side, [`ConditionError::NoAttribute`] when the operator needs a
/// target attribute the condition does not name, and
/// [`ConditionError::Read`] when a path does not read over the document.
pub fn evaluate<T: Table + ?Sized>(
    table: &T,
    document: &Value,
    condition: &Condition,
    attached: bool,
) -> Result<Verdict, ConditionError> {
    let root = fhir_target(condition.target())?;
    let attributes = attribute_paths(condition)?;
    if !attached {
        // NOTE: Conditions.adoc, an unattached condition is "handled as simple
        // true/false", so the whole path answers once rather than per
        // occurrence of a root the `with` path does not share.
        let whole = Occurrence::default();
        return Ok(Verdict::Gate(holds(
            table,
            document,
            condition,
            &attributes,
            &whole,
        )));
    }
    let admitted = matches_of(table, document, root)?
        .into_iter()
        .filter(|occurrence| holds(table, document, condition, &attributes, occurrence))
        .collect();
    Ok(Verdict::Filter(admitted))
}

/// Returns the resolved FHIR expression a condition target names.
fn fhir_target(
    target: &crate::resolve::program::target::Target,
) -> Result<&FhirPath, ConditionError> {
    match *target {
        crate::resolve::program::target::Target::Fhir(ref target) => Ok(target.expression()),
        crate::resolve::program::target::Target::Openehr(_) => Err(ConditionError::WrongSide),
    }
}

/// Returns the resolved attribute expressions, refusing an operator with none.
fn attribute_paths(condition: &Condition) -> Result<Vec<&FhirPath>, ConditionError> {
    let mut paths = Vec::with_capacity(condition.attributes().len());
    for attribute in condition.attributes() {
        paths.push(fhir_target(attribute)?);
    }
    // NOTE: Conditions.adoc §targetAttribute makes the attribute the path the
    // filter applies to, so only `type` can stand on the root alone.
    if paths.is_empty() && condition.operator() != ConditionOperator::Type {
        return Err(ConditionError::NoAttribute {
            operator: condition.operator(),
        });
    }
    Ok(paths)
}

/// Returns the occurrences a path matches, with no deferred reference.
fn matches_of<T: Table + ?Sized>(
    table: &T,
    document: &Value,
    path: &FhirPath,
) -> Result<Vec<Occurrence>, ConditionError> {
    Ok(read_path(table, document, path)?
        .into_iter()
        .map(|matched| matched.occurrence().clone())
        .collect())
}

/// Reads a path, naming the expression in any refusal.
fn read_path<'tree, T: Table + ?Sized>(
    table: &T,
    document: &'tree Value,
    path: &FhirPath,
) -> Result<Vec<Match<'tree>>, ConditionError> {
    read(table, document, path).map_err(|source| ConditionError::Read {
        target: String::from(path.as_str()),
        source,
    })
}

/// Whether the condition holds for one occurrence of its target root.
fn holds<T: Table + ?Sized>(
    table: &T,
    document: &Value,
    condition: &Condition,
    attributes: &[&FhirPath],
    root: &Occurrence,
) -> bool {
    let mut found = attribute_values(table, document, attributes, root);
    if condition.operator() == ConditionOperator::Type {
        found.types = type_names(table, document, condition, attributes, root);
    }
    decide(condition, &found)
}

/// Returns whether `condition` holds over what its attributes found.
///
/// The connectives are the ones the page's operator table fixes: `one of` is
/// an OR over the criteria, `not of` an AND, `empty` and `not empty` ask
/// whether the path holds anything, and `type` is an OR over the named
/// classes.
#[must_use]
pub fn decide(condition: &Condition, found: &Attributes) -> bool {
    match condition.operator() {
        ConditionOperator::OneOf => found
            .values
            .iter()
            .any(|value| condition.criteria().iter().any(|wanted| wanted == value)),
        ConditionOperator::NotOf => found
            .values
            .iter()
            .all(|value| condition.criteria().iter().all(|wanted| wanted != value)),
        // NOTE: Conditions.adoc §empty asks whether the path is empty, and its
        // own example tests `bodysite.coding`, an object, so presence is what
        // answers rather than the scalar text a criteria list compares.
        ConditionOperator::Empty => found.present == 0,
        ConditionOperator::NotEmpty => found.present > 0,
        ConditionOperator::Type => found
            .types
            .iter()
            .any(|named| condition.criteria().iter().any(|wanted| wanted == named)),
    }
}

/// What the attribute paths of a condition hold under one root occurrence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes {
    /// How many values the paths matched, of any shape.
    pub present: usize,
    /// The scalar text of those values, for the criteria comparison.
    pub values: Vec<String>,
    /// The class names the `type` operator tests against.
    pub types: Vec<String>,
}

/// Returns what the attribute paths hold under one root occurrence.
fn attribute_values<T: Table + ?Sized>(
    table: &T,
    document: &Value,
    attributes: &[&FhirPath],
    root: &Occurrence,
) -> Attributes {
    let mut found = Attributes::default();
    for path in attributes {
        let Ok(matches) = read(table, document, path) else {
            continue;
        };
        for matched in matches {
            if !under(matched.occurrence(), root) {
                continue;
            }
            found.present = found.present.saturating_add(1);
            if let Selected::Value(value) = *matched.selected()
                && let Some(text) = scalar(value)
            {
                found.values.push(text);
            }
        }
    }
    found
}

/// Returns the element type names the `type` operator tests against.
///
/// "Checks if the element in the path matches the given type in `criteria`",
/// so the answer is the type the element table resolved the path to.
fn type_names<T: Table + ?Sized>(
    table: &T,
    document: &Value,
    condition: &Condition,
    attributes: &[&FhirPath],
    root: &Occurrence,
) -> Vec<String> {
    let mut paths: Vec<&FhirPath> = attributes.to_vec();
    if paths.is_empty()
        && let Ok(target) = fhir_target(condition.target())
    {
        paths.push(target);
    }
    let mut names = Vec::new();
    for path in paths {
        let Some(resource) = document.get("resourceType").and_then(Value::as_str) else {
            continue;
        };
        let Ok(resolved) = crate::tree::element::resolve(table, resource, path) else {
            continue;
        };
        let Ok(matches) = read(table, document, path) else {
            continue;
        };
        if !matches
            .iter()
            .any(|matched| under(matched.occurrence(), root))
        {
            continue;
        }
        if let Some(name) = type_name(resolved.location()) {
            names.push(String::from(name));
        }
    }
    names
}

/// Returns the FHIR type name a resolved location stands for.
const fn type_name(location: &Location) -> Option<&'static str> {
    match *location {
        Location::Complex(schema) => Some(schema.path),
        Location::Primitive(_)
        | Location::Attribute
        | Location::PrimitiveElement
        | Location::Choice(_)
        | Location::Resource
        | Location::Deferred => None,
    }
}

/// Whether `occurrence` lies under `root`.
///
/// A condition's attribute path extends its target root, so a match belongs to
/// the root occurrence whose indices its own begin with.
fn under(occurrence: &Occurrence, root: &Occurrence) -> bool {
    occurrence.indices().starts_with(root.indices())
}

/// Returns the text a scalar value carries, `None` for a structure.
///
/// A criteria list is text, so a boolean and a number are compared in their
/// lexical form (<https://hl7.org/fhir/R4/json.html>); an object or an array
/// is no criterion.
fn scalar(value: &Value) -> Option<String> {
    match *value {
        Value::String(ref text) => Some(text.clone()),
        Value::Bool(flag) => Some(String::from(if flag { "true" } else { "false" })),
        Value::Number(ref number) => Some(String::from(number.as_str())),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}
