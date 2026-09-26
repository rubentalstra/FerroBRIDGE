// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The element-table view of a path.
//!
//! A path expression names elements, and what those elements are is the FHIR
//! `ElementDefinition` set of the version in play
//! (<https://hl7.org/fhir/R4/elementdefinition.html>). `fhir-types` emits that
//! set per version as `schema::SCHEMAS`, with each element's path, cardinality,
//! type codes and content reference, so this module resolves against it and
//! models nothing of FHIR itself.
//!
//! Resolution turns a [`crate::tree::path::FhirPath`] into the [`Move`]
//! sequence a document walk follows. A type filter is not a move of its own: on
//! a choice element it fixes the JSON key
//! (<https://hl7.org/fhir/R4/json.html>), on an element that already has the
//! named type it disappears, and on a reference it becomes the deferred
//! resolution the engine finishes.

pub mod walk;

use fhir_types::schema::Kind;
use fhir_types::schema::Schemas;
use fhir_types::schema::TypeSchema;
use fhir_types::schema::ValueKind;

use crate::tree::error::ResolveError;
use crate::tree::path::FhirPath;
use crate::tree::path::Head;
use crate::tree::path::Ordinal;
use crate::tree::path::Step;

/// The element table of one FHIR version.
///
/// `fhir-types` emits one `Schemas` per version and this trait is what the
/// path model asks of it, so a later version plugs in without touching the
/// resolver.
pub trait Table {
    /// Returns the schema of the type named `name`.
    fn type_named(&self, name: &str) -> Option<&'static TypeSchema>;

    /// Returns whether `name` is a resource type of this version.
    fn is_resource(&self, name: &str) -> bool;
}

impl Table for Schemas {
    fn type_named(&self, name: &str) -> Option<&'static TypeSchema> {
        Self::type_named(self, name)
    }

    fn is_resource(&self, name: &str) -> bool {
        Self::is_resource(self, name)
    }
}

/// One element a resolved path steps into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    path: String,
    key: String,
    kind: Kind,
    min: u32,
    max: Option<u32>,
    types: &'static [&'static str],
}

impl Field {
    /// Returns the element path the table holds, for example
    /// `Condition.onset[x]`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the JSON key this element is written under.
    ///
    /// A choice element's key carries the type suffix and a primitive's
    /// extension sibling carries the leading underscore
    /// (<https://hl7.org/fhir/R4/json.html>).
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns how the element travels.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Returns the minimum cardinality.
    #[must_use]
    pub const fn min(&self) -> u32 {
        self.min
    }

    /// Returns the maximum cardinality, `None` for `*`.
    #[must_use]
    pub const fn max(&self) -> Option<u32> {
        self.max
    }

    /// Returns the type codes the definition lists, in its order.
    ///
    /// A choice element lists every alternative and a content reference lists
    /// none (<https://hl7.org/fhir/R4/elementdefinition.html>).
    #[must_use]
    pub const fn types(&self) -> &'static [&'static str] {
        self.types
    }

    /// Returns whether the element repeats, so the JSON holds an array.
    #[must_use]
    pub fn repeats(&self) -> bool {
        self.max != Some(1)
    }

    /// The same element under another JSON key.
    fn under(&self, key: String) -> Self {
        Self {
            path: self.path.clone(),
            key,
            kind: self.kind,
            min: self.min,
            max: self.max,
            types: self.types,
        }
    }
}

/// One move of a document walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Move {
    /// Into the object member the element is written under.
    Member(Field),
    /// Into whichever suffixed member of a choice element the document
    /// carries.
    Choice {
        /// The choice element, keyed by its stem name.
        field: Field,
        /// The suffix and kind of each alternative the definition lists.
        variants: &'static [(&'static str, Kind)],
    },
    /// Into the `extension` list, taking the entry with this url.
    ///
    /// `extension(url)` "will filter the input collection for items named
    /// 'extension' with the given url" (<https://hl7.org/fhir/R4/fhirpath.html>,
    /// §Additional functions).
    Extension {
        /// The `extension` element itself.
        field: Field,
        /// The url the literal carried.
        url: String,
    },
    /// One end of the values a repeating element already holds.
    Ordinal(Ordinal),
    /// One position of the values a repeating element already holds.
    Index(usize),
    /// A FHIRPath predicate.
    Predicate(String),
    /// A reference the engine resolves, with what remains of the path.
    Resolve {
        /// The resource type a following type filter asserted.
        expected: Option<String>,
        /// The steps to apply to the resolved resource.
        continuation: Vec<Step>,
    },
}

/// Where a resolved path ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// Inside a resource, a datatype or a backbone element.
    Complex(&'static TypeSchema),
    /// On a primitive element.
    Primitive(ValueKind),
    /// On a plain string element, `Element.id` or `Extension.url`.
    Attribute,
    /// On the sibling object that carries a primitive's `id` and `extension`.
    PrimitiveElement,
    /// On a choice element no type filter has resolved.
    Choice(String),
    /// On an element whose resource type only the document names.
    Resource,
    /// On a reference the engine resolves.
    Deferred,
}

/// A path resolved against the element table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    resource: String,
    moves: Vec<Move>,
    location: Location,
}

impl Resolved {
    /// Returns the resource type the path was resolved against.
    #[must_use]
    pub fn resource(&self) -> &str {
        &self.resource
    }

    /// Returns the moves a document walk follows, in path order.
    #[must_use]
    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// Returns where the path ends.
    #[must_use]
    pub const fn location(&self) -> &Location {
        &self.location
    }

    /// Returns the FHIR type code of the element the walk ends on.
    ///
    /// A complex element names its type, and a primitive the one code its
    /// definition lists (<https://hl7.org/fhir/R4/elementdefinition.html>,
    /// `ElementDefinition.type.code`). `None` for a choice no filter
    /// resolved, a resource, a deferred reference and the sibling object of a
    /// primitive, none of which has one type.
    #[must_use]
    pub fn type_code(&self) -> Option<&'static str> {
        match self.location {
            Location::Complex(schema) => Some(schema.name),
            Location::Primitive(_) | Location::Attribute => match self.moves.last() {
                Some(Move::Member(field) | Move::Extension { field, .. }) => match field.types {
                    [code] => Some(*code),
                    _ => None,
                },
                _ => None,
            },
            Location::PrimitiveElement
            | Location::Choice(_)
            | Location::Resource
            | Location::Deferred => None,
        }
    }

    /// Returns the element path the walk ends on, the resource type when the
    /// path has no element step.
    #[must_use]
    pub fn leaf(&self) -> &str {
        self.moves
            .iter()
            .rev()
            .find_map(|step| match step {
                Move::Member(field)
                | Move::Choice { field, .. }
                | Move::Extension { field, .. } => Some(field.path()),
                Move::Ordinal(_) | Move::Index(_) | Move::Predicate(_) | Move::Resolve { .. } => {
                    None
                }
            })
            .unwrap_or(&self.resource)
    }
}

/// Resolves `path` against `table`, starting at the type `resource` names.
///
/// The path must be rooted at `$resource`; bind a relative one to its anchor
/// with [`FhirPath::anchored`] first.
///
/// # Errors
///
/// Returns [`ResolveError`] naming the element path from the table for an
/// unknown element, a type filter the element does not admit, a step through a
/// choice no filter resolved, or an index on an element that does not repeat.
pub fn resolve<T: Table + ?Sized>(
    table: &T,
    resource: &str,
    path: &FhirPath,
) -> Result<Resolved, ResolveError> {
    if path.head() != Head::Resource {
        return Err(ResolveError::NotAnchored {
            expression: String::from(path.as_str()),
        });
    }
    let root = table
        .type_named(resource)
        .filter(|schema| schema.path == resource && table.is_resource(resource))
        .ok_or_else(|| ResolveError::UnknownResource {
            name: String::from(resource),
        })?;
    let mut walk = Walk {
        table,
        location: Location::Complex(root),
        owner: String::from(resource),
        moves: Vec::new(),
    };
    let steps = path.steps();
    let mut index = 0usize;
    while let Some(step) = steps.get(index) {
        index = index.saturating_add(1);
        if walk.take(step, steps.get(index..).unwrap_or_default())? {
            break;
        }
    }
    Ok(Resolved {
        resource: String::from(resource),
        moves: walk.moves,
        location: walk.location,
    })
}

/// The resolver's state as it walks the steps.
struct Walk<'table, T: Table + ?Sized> {
    table: &'table T,
    location: Location,
    owner: String,
    moves: Vec<Move>,
}
