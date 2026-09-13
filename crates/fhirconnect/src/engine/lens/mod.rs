// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The data-type converters, written once and run both ways.
//!
//! FHIRconnect declares its mappings bidirectional, so a converter written
//! twice is two chances to disagree. Each cell of the specification's
//! data-type chapter
//! (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/`)
//! is one [`Lens`] instead: [`Lens::get`] reads the openEHR value into the
//! FHIR element and [`Lens::put`] writes the FHIR element back, and the two
//! are tested against each other as the well-behaved-lens laws (`GetPut` and
//! `PutGet`, Weber and Ho, J Healthc Inform Res 2020,
//! <https://doi.org/10.1007/s41666-019-00065-0>).
//!
//! # What a lens may drop and what it must refuse
//!
//! Every data-type table marks some attributes `-` on one side: the other
//! model has no counterpart for them. Two things follow, and they are
//! different.
//!
//! An attribute the table declares to have no counterpart is **carried, not
//! refused**. [`Lens::put`] takes the openEHR value the target already holds,
//! so the attributes FHIR cannot express survive the round trip through the
//! composition. The CDR is the system of record
//! (`docs/architecture.md` §4.6), and `DV_TEXT.language` is not an element
//! the program failed to map: it is an attribute the cell declares FHIR does
//! not have.
//!
//! A value whose meaning the target would **falsify** is refused with a typed
//! [`LensError`]. A `TERM_MAPPING` whose `match` is `<` says the target term
//! is narrower; rendering it as a plain `Coding` beside the defining one
//! asserts an equivalence the data denies, so it is refused rather than
//! flattened. This is the strict-coercion rule of `docs/architecture.md`
//! §4.4, and the line between the two cases is FerroBRIDGE's own: no
//! specification governs it.

pub mod code_phrase;
pub mod coded_text;
pub mod date_time;
pub mod interval;
pub mod party;
pub mod proportion;
pub mod term_mapping;
pub mod text;

use core::fmt;

use crate::model::ast::Direction;

/// One data-type cell of the specification's chapter, run both ways.
///
/// `O` is the openEHR reference-model value and `F` the FHIR element. One
/// type may carry several cells, which is why the two sides are parameters
/// rather than associated types: `DV_TEXT` has a `string` cell, a `Coding`
/// cell and a `CodeableConcept` cell, and the same lens implements all three.
pub trait Lens<O, F> {
    /// Names the cell, for a diagnostic and for the recorded fallbacks.
    fn cell(&self) -> &'static str;

    /// Reads the openEHR value into the FHIR element.
    ///
    /// # Errors
    ///
    /// Returns [`LensError`] when the value cannot be carried into the FHIR
    /// element without falsifying it.
    fn get(&self, source: &O) -> Result<F, LensError>;

    /// Writes the FHIR element back into an openEHR value.
    ///
    /// `existing` is the value the target already holds, when the mapping
    /// overwrites one. The attributes the cell declares FHIR has no
    /// counterpart for are taken from it, which is what makes the round trip
    /// through a composition lossless.
    ///
    /// # Errors
    ///
    /// Returns [`LensError`] when the element does not carry what the openEHR
    /// value requires, or when the cell is not defined in this direction.
    fn put(&self, view: &F, existing: Option<&O>) -> Result<O, LensError>;

    /// The one-way fallbacks [`Lens::put`] takes for this element.
    ///
    /// A fallback is a rule the specification writes for one direction only,
    /// so the value it produces does not return through [`Lens::get`]. The
    /// engine records each one in its outcome rather than leaving the loss
    /// silent (`docs/architecture.md` §4.4).
    fn fallbacks(&self, view: &F) -> Vec<Fallback>;
}

/// One rule a lens applied that runs in one direction only.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fallback {
    cell: &'static str,
    field: &'static str,
}

impl Fallback {
    /// Records a fallback taken in `cell`, filling `field`.
    #[must_use]
    pub const fn new(cell: &'static str, field: &'static str) -> Self {
        Self { cell, field }
    }

    /// Returns the cell the fallback belongs to.
    #[must_use]
    pub const fn cell(&self) -> &'static str {
        self.cell
    }

    /// Returns the attribute the fallback filled.
    #[must_use]
    pub const fn field(&self) -> &'static str {
        self.field
    }
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} filled {}", self.cell, self.field)
    }
}

/// Why a data-type cell refused a value.
///
/// Every variant names the cell and the attribute, so a diagnostic points at
/// the row of the specification table that governs it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LensError {
    /// The source carries nothing for an attribute the target requires.
    #[error("{cell} going {direction} needs {attribute}, which the source does not carry")]
    Missing {
        /// The cell that refused.
        cell: &'static str,
        /// The attribute of the target that has no source.
        attribute: &'static str,
        /// The direction the cell was run in.
        direction: Direction,
    },
    /// The source carries a shape the target cannot hold.
    #[error("{cell} going {direction} cannot carry {attribute} {found}: {reason}")]
    Shape {
        /// The cell that refused.
        cell: &'static str,
        /// The attribute whose value refused.
        attribute: &'static str,
        /// The value as the diagnostic renders it.
        found: String,
        /// Why the target cannot hold it.
        reason: &'static str,
        /// The direction the cell was run in.
        direction: Direction,
    },
    /// The specification defines the cell in one direction only.
    #[error("{cell} is defined going {defined} only, and was run going {direction}")]
    OneWay {
        /// The cell that refused.
        cell: &'static str,
        /// The direction the specification defines.
        defined: Direction,
        /// The direction the engine ran.
        direction: Direction,
    },
    /// The specification leaves the cell undefined for this input.
    #[error("{cell} going {direction} is not defined for {situation}: {reason}")]
    Undefined {
        /// The cell that refused.
        cell: &'static str,
        /// The input the specification does not resolve.
        situation: &'static str,
        /// What the specification says, and what it leaves open.
        reason: &'static str,
        /// The direction the cell was run in.
        direction: Direction,
    },
    /// A `DV_PROPORTION` that is not a percentage, with no named carrier.
    ///
    /// FHIR `Quantity` has no denominator, so only `pk_percent` has a
    /// faithful representation (`docs/architecture.md` §4.4). The engine
    /// never writes the reference engine's `proportion-denominator` and
    /// `proportion-kind` extension URLs, which no specification defines.
    #[error(
        "a DV_PROPORTION of kind {kind} has no faithful FHIR Quantity; \
         name a carrier that holds a denominator"
    )]
    UnrepresentableProportion {
        /// The `PROPORTION_KIND` the value carries.
        kind: i32,
    },
}
