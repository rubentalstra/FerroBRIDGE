// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The resolved sides of a mapping: the FHIR element and the openEHR node.

use core::fmt;

use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::paths::FlatId;
use openehr_rm::v1_2::paths::RmPath;

use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::Resolved;
use crate::tree::path::FhirPath;
use crate::tree::path::Writability;

/// A `with.fhir` expression, bound to its anchor and resolved against the
/// element table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FhirTarget {
    expression: FhirPath,
    resolved: Resolved,
}

impl FhirTarget {
    /// Pairs an anchored expression with its element-table resolution.
    #[must_use]
    pub const fn new(expression: FhirPath, resolved: Resolved) -> Self {
        Self {
            expression,
            resolved,
        }
    }

    /// Returns the `$resource`-rooted expression.
    #[must_use]
    pub const fn expression(&self) -> &FhirPath {
        &self.expression
    }

    /// Returns the element-table resolution.
    #[must_use]
    pub const fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    /// Returns whether the expression can be written through.
    #[must_use]
    pub const fn writability(&self) -> &Writability {
        self.expression.writability()
    }

    /// Returns whether the expression names many values.
    ///
    /// The element the walk ends on decides it, unless a later step picks one
    /// of the values a document already holds: an index, an ordinal, a
    /// predicate, or the `extension(url)` shortcut, whose url "is the
    /// identity" of one extension
    /// (<https://hl7.org/fhir/R4/extensibility.html>). This is the FHIR-side
    /// counterpart of [`OpenehrTarget::occurrences`].
    #[must_use]
    pub fn repeats(&self) -> bool {
        let mut repeats = false;
        for step in self.resolved.moves() {
            match *step {
                Move::Member(ref field) | Move::Choice { ref field, .. } => {
                    repeats = field.repeats();
                }
                Move::Extension { .. } | Move::Ordinal(_) | Move::Index(_) | Move::Predicate(_) => {
                    repeats = false;
                }
                Move::Resolve { .. } => {}
            }
        }
        repeats
    }
}

impl fmt::Display for FhirTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let writability = match *self.writability() {
            Writability::Writable => "writable",
            Writability::ReadOnly { .. } => "read-only",
        };
        write!(
            f,
            "{} -> {} {} {}",
            self.expression,
            self.resolved.leaf(),
            location_name(self.resolved.location()),
            writability
        )
    }
}

/// Names where a resolved FHIR path ends, for a rendering that stays stable.
fn location_name(location: &Location) -> &'static str {
    match *location {
        Location::Complex(_) => "complex",
        Location::Primitive(_) => "primitive",
        Location::Attribute => "attribute",
        Location::PrimitiveElement => "primitive-element",
        Location::Choice(_) => "choice",
        Location::Resource => "resource",
        Location::Deferred => "deferred",
    }
}

/// A `with.openehr` path, bound to its anchor and resolved against the Web
/// Template.
///
/// The Web Template indexes the nodes a template constrains, and an openEHR
/// path may reach below the deepest of them into the reference model, so a
/// target carries the node plus the attribute tail under it. No specification
/// governs the meeting of a mapping path and a Web Template: this split is
/// FerroBRIDGE's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenehrTarget {
    path: RmPath,
    node: ResolvedNode,
    tail: RmPath,
    occurrences: Vec<FlatId>,
    leaf_class: Option<String>,
}

impl OpenehrTarget {
    /// Pairs a resolved path with the template node it names.
    #[must_use]
    pub const fn new(
        path: RmPath,
        node: ResolvedNode,
        tail: RmPath,
        occurrences: Vec<FlatId>,
    ) -> Self {
        Self {
            path,
            node,
            tail,
            occurrences,
            leaf_class: None,
        }
    }

    /// Returns this target with `class` as the reference-model class of the
    /// last attribute its tail names.
    #[must_use]
    pub fn with_leaf_class(mut self, class: impl Into<String>) -> Self {
        self.leaf_class = Some(class.into());
        self
    }

    /// Returns the reference-model class of the last attribute the tail names,
    /// `None` for a path that names the node itself.
    ///
    /// The resolver reads it from the RM attribute model
    /// (<https://docs.rs/openehr-rm/0.0.69/openehr_rm/v1_2/model/fn.attribute.html>),
    /// and it is the class, in place of the node's, whose value the engine
    /// reads and writes at the end of the tail.
    #[must_use]
    pub fn leaf_class(&self) -> Option<&str> {
        self.leaf_class.as_deref()
    }

    /// Returns the absolute openEHR path the mapping names.
    #[must_use]
    pub const fn path(&self) -> &RmPath {
        &self.path
    }

    /// Returns the template node the path resolves to.
    #[must_use]
    pub const fn node(&self) -> &ResolvedNode {
        &self.node
    }

    /// Returns the reference-model attributes below the node, empty when the
    /// path names the node itself.
    #[must_use]
    pub const fn tail(&self) -> &RmPath {
        &self.tail
    }

    /// Returns the repeating nodes on the way to this one, outermost first.
    ///
    /// One occurrence index belongs to each of them, which is what makes an
    /// occurrence a structured index rather than a pattern over a rendered
    /// path.
    #[must_use]
    pub fn occurrences(&self) -> &[FlatId] {
        &self.occurrences
    }
}

impl fmt::Display for OpenehrTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} -> {} {}",
            self.path,
            self.node.aql_path().as_str(),
            self.node.rm_type()
        )?;
        if !self.tail.segments.is_empty() {
            write!(f, " tail {}", self.tail)?;
        }
        if !self.occurrences.is_empty() {
            let axes: Vec<&str> = self.occurrences.iter().map(FlatId::as_str).collect();
            write!(f, " occurrences [{}]", axes.join(", "))?;
        }
        Ok(())
    }
}

/// One side of a mapping, on the side the path belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A FHIR expression.
    Fhir(Box<FhirTarget>),
    /// An openEHR path.
    Openehr(Box<OpenehrTarget>),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Fhir(ref target) => write!(f, "fhir {target}"),
            Self::Openehr(ref target) => write!(f, "openehr {target}"),
        }
    }
}

/// How a condition's `targetRoot` stands to the path it guards.
///
/// The `targetRoot` "defines what element is returned once filtered ... the
/// returned element is matched against the path in the `with` method", and a
/// condition may also "be unattached to the path contained in the `with:`
/// method and point to a different path", which is "handled as simple
/// true/false"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §targetRoot). The compiler decides which of the four it is, over the
/// anchored paths, so the engine never compares path text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attachment {
    /// The `targetRoot` is the guarded path, so the condition filters the
    /// values that path maps.
    Element,
    /// The `targetRoot` is above the guarded path.
    Ancestor,
    /// The `targetRoot` is below the guarded path, which is the shape a
    /// preprocessor gate writes and a mapping-level condition is refused for.
    Descendant,
    /// Neither path contains the other, so the condition is a plain true or
    /// false test.
    Unrelated,
}

impl fmt::Display for Attachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Element => f.write_str("element"),
            Self::Ancestor => f.write_str("ancestor"),
            Self::Descendant => f.write_str("descendant"),
            Self::Unrelated => f.write_str("unrelated"),
        }
    }
}
