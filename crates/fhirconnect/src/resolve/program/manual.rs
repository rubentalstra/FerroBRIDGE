// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A resolved `manual` entry and its paths.

use core::fmt;

use crate::model::ast::keyword::Direction;

use crate::resolve::program::condition::Condition;
use crate::resolve::program::condition::render_condition;
use crate::resolve::program::target::Target;

/// What a `manual` entry writes at a resolved path.
///
/// A value is a literal unless it names a `$context` member, which "holds
/// values passed in on the REST call ... a context value is referenced from a
/// `manual` `value`"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`,
/// §`$context`), so the two are told apart when the mapping is compiled rather
/// than on every request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ManualValue {
    /// The text the mapping wrote.
    Literal(String),
    /// The name of the `$context` member the caller supplies.
    Context(String),
}

impl fmt::Display for ManualValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Literal(ref text) => f.write_str(text),
            Self::Context(ref name) => write!(f, "$context.{name}"),
        }
    }
}

/// One value a `manual` entry writes at a resolved path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPath {
    target: Target,
    value: ManualValue,
}

impl ManualPath {
    /// Pairs a resolved path with the value written there.
    #[must_use]
    pub const fn new(target: Target, value: ManualValue) -> Self {
        Self { target, value }
    }

    /// Returns the resolved path.
    #[must_use]
    pub const fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the value.
    #[must_use]
    pub const fn value(&self) -> &ManualValue {
        &self.value
    }
}

/// A compiled `manual` entry.
///
/// The paths inside one entry merge into one element
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/manual.adoc`),
/// so they travel together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manual {
    name: String,
    fhir: Vec<ManualPath>,
    openehr: Vec<ManualPath>,
    fhir_condition: Option<Condition>,
    openehr_condition: Option<Condition>,
    value: Option<String>,
    direction: Option<Direction>,
}

impl Manual {
    /// Assembles a compiled manual entry.
    #[must_use]
    pub const fn new(
        name: String,
        fhir: Vec<ManualPath>,
        openehr: Vec<ManualPath>,
        fhir_condition: Option<Condition>,
        openehr_condition: Option<Condition>,
        value: Option<String>,
        direction: Option<Direction>,
    ) -> Self {
        Self {
            name,
            fhir,
            openehr,
            fhir_condition,
            openehr_condition,
            value,
            direction,
        }
    }

    /// Returns the entry name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the FHIR literals of the entry.
    #[must_use]
    pub fn fhir(&self) -> &[ManualPath] {
        &self.fhir
    }

    /// Returns the openEHR literals of the entry.
    #[must_use]
    pub fn openehr(&self) -> &[ManualPath] {
        &self.openehr
    }

    /// Returns the condition the FHIR to openEHR direction evaluates.
    #[must_use]
    pub const fn fhir_condition(&self) -> Option<&Condition> {
        self.fhir_condition.as_ref()
    }

    /// Returns the condition the openEHR to FHIR direction evaluates.
    #[must_use]
    pub const fn openehr_condition(&self) -> Option<&Condition> {
        self.openehr_condition.as_ref()
    }

    /// Returns the value the entry writes, when it writes one value rather
    /// than a set of paths.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Returns the one direction the entry runs in, when it is pinned to one.
    #[must_use]
    pub const fn direction(&self) -> Option<Direction> {
        self.direction
    }
}

/// Writes one compiled manual entry.
pub(super) fn render_manual(f: &mut fmt::Formatter<'_>, pad: &str, manual: &Manual) -> fmt::Result {
    writeln!(f, "{pad}manual {}", manual.name())?;
    let inner = format!("{pad}  ");
    if let Some(direction) = manual.direction() {
        writeln!(f, "{inner}unidirectional {direction}")?;
    }
    if let Some(value) = manual.value() {
        writeln!(f, "{inner}value {value}")?;
    }
    for path in manual.fhir().iter().chain(manual.openehr()) {
        writeln!(f, "{inner}{} = {}", path.target(), path.value())?;
    }
    for (label, condition) in [
        ("fhirCondition", manual.fhir_condition()),
        ("openehrCondition", manual.openehr_condition()),
    ] {
        if let Some(condition) = condition {
            render_condition(f, &inner, label, condition)?;
        }
    }
    Ok(())
}
