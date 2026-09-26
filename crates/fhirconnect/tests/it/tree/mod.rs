// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The tree module against the R4 element table.
//!
//! Read and write go through the public API over synthetic documents built in
//! the test. Every fixture value is invented here; no clinical content of any
//! kind appears in this file.

mod diagnose;
mod element;
mod laws;
mod path;
mod refusals;

use core::error::Error;

use fhir_types::codec::Object;
use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::tree::Occurrence;
use fhirconnect::tree::element::Move;
use fhirconnect::tree::element::resolve;
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::read::Match;
use fhirconnect::tree::read::read;
use fhirconnect::tree::write::write;

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
