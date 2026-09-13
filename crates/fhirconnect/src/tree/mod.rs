// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The bidirectional path model over FHIR JSON.
//!
//! A FHIRconnect mapping writes one path per side and runs both ways, so
//! `with.fhir` is read and written through the same expression. Two of the
//! forms it uses are not FHIRPath at all, `$fhirRoot` and the `^` parent
//! operator (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/path_operators.html>),
//! so this module parses the expression itself, resolves it against the FHIR
//! element table, and evaluates it over the lexical `Value` tree of
//! `fhir-types`.
//!
//! The parts are separate modules:
//!
//! - [`path`] parses an expression into a [`path::FhirPath`] and classifies it
//!   as writable or read-only.
//! - [`element`] resolves a path against the element table of one FHIR
//!   version, naming every element it traverses.
//! - [`read`] evaluates a resolved path over a document.
//! - [`write`] sets a value at a writable path, creating what the element
//!   table's cardinality says to create.
//! - [`error`] holds the error types the four steps return.
//!
//! The JSON shape is the FHIR JSON representation
//! (<https://hl7.org/fhir/R4/json.html>): a repeating element is an array, a
//! choice element is the stem followed by the type name, and a primitive's
//! `id` and `extension` live in the sibling member named with a leading
//! underscore.

pub mod element;
pub mod error;
pub mod path;
pub mod read;
pub mod write;

use core::fmt;

use fhir_types::codec::Value;

/// Names the JSON kind a value has, for a diagnostic that reports a shape.
const fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The array index taken at each repeating element along a path, outermost
/// first.
///
/// FHIRconnect pairs a FHIR occurrence with an openEHR occurrence, and the
/// pairing is a structured index rather than a pattern over a rendered path
/// (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/recurrence/main.html>).
/// [`read::read`] returns the index of every match and [`write::write`] takes
/// the index to write at, so one element of the sequence belongs to each
/// repeating element the path steps through, in path order.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Occurrence(Vec<usize>);

impl Occurrence {
    /// Creates an occurrence from its indices, outermost first.
    #[must_use]
    pub fn new(indices: impl Into<Vec<usize>>) -> Self {
        Self(indices.into())
    }

    /// Returns the indices, outermost first.
    #[must_use]
    pub fn indices(&self) -> &[usize] {
        &self.0
    }

    /// Returns how many repeating elements the occurrence covers.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the occurrence covers no repeating element.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends one index.
    fn push(&mut self, index: usize) {
        self.0.push(index);
    }
}

impl fmt::Display for Occurrence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for index in &self.0 {
            if !first {
                f.write_str(".")?;
            }
            write!(f, "{index}")?;
            first = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Occurrence;

    #[test]
    fn an_occurrence_renders_its_indices_in_path_order() {
        assert_eq!(Occurrence::new([0, 2, 1]).to_string(), "0.2.1");
        assert_eq!(Occurrence::default().to_string(), "");
        assert_eq!(Occurrence::new([3]).depth(), 1);
        assert!(Occurrence::default().is_empty());
    }
}
