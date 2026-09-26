// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The walk of one expression over the element table, one step at a time.

use fhir_types::schema::FieldSchema;
use fhir_types::schema::Kind;
use fhir_types::schema::TypeSchema;
use fhir_types::schema::ValueKind;

use crate::tree::error::ResolveError;
use crate::tree::path::Step;

use crate::tree::element::Field;
use crate::tree::element::Location;
use crate::tree::element::Move;
use crate::tree::element::Table;
use crate::tree::element::Walk;

/// The primitive type codes `resolve()` accepts.
///
/// `resolve()` reads "a string that is a uri (or canonical or url)"
/// (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions).
const REFERENTIAL: [&str; 3] = ["uri", "canonical", "url"];

/// The table entry a primitive's underscore sibling resolves against.
///
/// A primitive's `id` and `extension` live in the member named with a leading
/// underscore (<https://hl7.org/fhir/R4/json.html>), and both come from
/// `Element` itself (<https://hl7.org/fhir/R4/element.html>), which the table
/// carries as its own type.
const ELEMENT_TYPE: &str = "Element";

/// The element name of the extension list every element carries.
const EXTENSION_ELEMENT: &str = "extension";

impl<T: Table + ?Sized> Walk<'_, T> {
    /// Takes one step, answering whether the walk is finished.
    pub(super) fn take(&mut self, step: &Step, rest: &[Step]) -> Result<bool, ResolveError> {
        match step {
            Step::Element { name } => self.element(name).map(|()| false),
            Step::Type { name, .. } => self.type_filter(name, rest),
            Step::Extension { url } => self.extension(url).map(|()| false),
            Step::Resolve => self.resolve_step(rest).map(|()| true),
            Step::Ordinal(ordinal) => {
                self.moves.push(Move::Ordinal(*ordinal));
                Ok(false)
            }
            Step::Index(position) => self.index(*position).map(|()| false),
            Step::Predicate { expression } => {
                self.moves.push(Move::Predicate(expression.clone()));
                Ok(false)
            }
        }
    }

    /// Steps into the element `name`.
    fn element(&mut self, name: &str) -> Result<(), ResolveError> {
        match &self.location {
            Location::Complex(schema) => self.member(schema, name),
            Location::Primitive(_) | Location::Attribute => {
                self.enter_primitive_element(name)?;
                self.primitive_member(name)
            }
            Location::PrimitiveElement => self.primitive_member(name),
            Location::Choice(element) => Err(ResolveError::UnresolvedChoice {
                element: element.clone(),
            }),
            Location::Resource => Err(ResolveError::ResourceContent {
                element: self.owner.clone(),
            }),
            Location::Deferred => Err(ResolveError::NoChildren {
                element: self.owner.clone(),
                name: String::from(name),
            }),
        }
    }

    /// Steps into a member of the complex type the walk stands in.
    fn member(&mut self, schema: &'static TypeSchema, name: &str) -> Result<(), ResolveError> {
        let (field, kind, key, types) =
            named(schema, name).ok_or_else(|| ResolveError::UnknownElement {
                owner: String::from(schema.path),
                name: String::from(name),
            })?;
        let resolved = Field {
            path: String::from(field.path),
            key,
            kind,
            min: field.min,
            max: field.max,
            types,
        };
        self.owner = String::from(field.path);
        if let Kind::Choice(variants) = kind {
            self.location = Location::Choice(String::from(field.path));
            self.moves.push(Move::Choice {
                field: resolved,
                variants,
            });
            return Ok(());
        }
        self.location = self.landing(kind)?;
        self.moves.push(Move::Member(resolved));
        Ok(())
    }

    /// Steps into the `id` or `extension` of a primitive's sibling object.
    fn primitive_member(&mut self, name: &str) -> Result<(), ResolveError> {
        let field = self.element_member(name)?;
        let path = format!("{}.{}", self.owner, field.name);
        self.owner.clone_from(&path);
        self.location = self.landing(field.kind)?;
        self.moves.push(Move::Member(Field {
            path,
            key: String::from(field.name),
            kind: field.kind,
            min: field.min,
            max: field.max,
            types: field.types,
        }));
        Ok(())
    }

    /// The `Element` member `name`, from the table's own `Element` entry.
    fn element_member(&self, name: &str) -> Result<&'static FieldSchema, ResolveError> {
        let schema =
            self.table
                .type_named(ELEMENT_TYPE)
                .ok_or_else(|| ResolveError::UnknownElement {
                    owner: self.owner.clone(),
                    name: String::from(ELEMENT_TYPE),
                })?;
        schema
            .fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| ResolveError::NoChildren {
                element: self.owner.clone(),
                name: String::from(name),
            })
    }

    /// Moves the last member onto the sibling that carries a primitive's `id`
    /// and `extension`.
    fn enter_primitive_element(&mut self, name: &str) -> Result<(), ResolveError> {
        let last = self
            .moves
            .last_mut()
            .ok_or_else(|| ResolveError::NoChildren {
                element: self.owner.clone(),
                name: String::from(name),
            })?;
        let Move::Member(field) = last else {
            return Err(ResolveError::NoChildren {
                element: self.owner.clone(),
                name: String::from(name),
            });
        };
        *field = field.under(format!("_{}", field.key));
        self.location = Location::PrimitiveElement;
        Ok(())
    }

    /// Applies `ofType()` or `as()`, answering whether the walk is finished.
    fn type_filter(&mut self, name: &str, rest: &[Step]) -> Result<bool, ResolveError> {
        match self.location.clone() {
            Location::Choice(element) => self.choose(&element, name).map(|()| false),
            Location::Complex(schema) => self.assert_complex(schema, name, rest),
            Location::Resource => self.assert_resource(name),
            Location::Primitive(_) | Location::Attribute => {
                self.assert_primitive(name).map(|()| false)
            }
            Location::PrimitiveElement | Location::Deferred => Err(ResolveError::TypeAssertion {
                element: self.owner.clone(),
                requested: String::from(name),
                actual: String::from("Element"),
            }),
        }
    }

    /// Fixes the alternative of the choice element the walk stands on.
    fn choose(&mut self, element: &str, name: &str) -> Result<(), ResolveError> {
        let Some(Move::Choice { field, variants }) = self.moves.last() else {
            return Err(ResolveError::UnresolvedChoice {
                element: String::from(element),
            });
        };
        let (suffix, kind) = variants
            .iter()
            .find(|(suffix, _)| suffix.eq_ignore_ascii_case(name))
            .ok_or_else(|| ResolveError::ChoiceType {
                element: String::from(element),
                requested: String::from(name),
                admitted: variants
                    .iter()
                    .map(|(suffix, _)| *suffix)
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;
        let types = alternative(field.types, suffix);
        let chosen = Field {
            path: field.path.clone(),
            key: format!("{}{suffix}", field.key),
            kind: *kind,
            min: field.min,
            max: field.max,
            types,
        };
        self.location = self.landing(*kind)?;
        let last = self.moves.len().saturating_sub(1);
        if let Some(slot) = self.moves.get_mut(last) {
            *slot = Move::Member(chosen);
        }
        Ok(())
    }

    /// Applies a type filter to the complex type the walk stands in.
    fn assert_complex(
        &mut self,
        schema: &'static TypeSchema,
        name: &str,
        rest: &[Step],
    ) -> Result<bool, ResolveError> {
        if schema.name.eq_ignore_ascii_case(name) {
            return Ok(false);
        }
        // NOTE: only `resolve()` reaches a reference target
        // (<https://hl7.org/fhir/R4/fhirpath.html>), so a resource type named
        // on a `Reference` is read as that same deferred resolution.
        if schema.name == "Reference" && self.table.is_resource(name) {
            self.defer(Some(String::from(name)), rest);
            return Ok(true);
        }
        Err(ResolveError::TypeAssertion {
            element: self.owner.clone(),
            requested: String::from(name),
            actual: String::from(schema.name),
        })
    }

    /// Applies a type filter to a resource-typed element.
    fn assert_resource(&mut self, name: &str) -> Result<bool, ResolveError> {
        let schema = self
            .table
            .type_named(name)
            .filter(|_| self.table.is_resource(name))
            .ok_or_else(|| ResolveError::TypeAssertion {
                element: self.owner.clone(),
                requested: String::from(name),
                actual: String::from("Resource"),
            })?;
        self.location = Location::Complex(schema);
        Ok(false)
    }

    /// Applies a type filter to a primitive element.
    fn assert_primitive(&mut self, name: &str) -> Result<(), ResolveError> {
        let Some(Move::Member(field)) = self.moves.last() else {
            return Err(ResolveError::TypeAssertion {
                element: self.owner.clone(),
                requested: String::from(name),
                actual: String::from("Element"),
            });
        };
        if field
            .types
            .iter()
            .any(|code| code.eq_ignore_ascii_case(name))
        {
            return Ok(());
        }
        Err(ResolveError::TypeAssertion {
            element: field.path.clone(),
            requested: String::from(name),
            actual: field.types.join(", "),
        })
    }

    /// Steps into the extension with `url`.
    fn extension(&mut self, url: &str) -> Result<(), ResolveError> {
        if matches!(self.location, Location::Primitive(_) | Location::Attribute) {
            self.enter_primitive_element(EXTENSION_ELEMENT)?;
        }
        let (field, path) = match &self.location {
            Location::Complex(schema) => {
                let (field, ..) = named(schema, EXTENSION_ELEMENT).ok_or_else(|| {
                    ResolveError::UnknownElement {
                        owner: String::from(schema.path),
                        name: String::from(EXTENSION_ELEMENT),
                    }
                })?;
                (field, String::from(field.path))
            }
            Location::PrimitiveElement => {
                let field = self.element_member(EXTENSION_ELEMENT)?;
                let path = format!("{}.{EXTENSION_ELEMENT}", self.owner);
                (field, path)
            }
            Location::Choice(choice) => {
                return Err(ResolveError::UnresolvedChoice {
                    element: choice.clone(),
                });
            }
            Location::Resource => {
                return Err(ResolveError::ResourceContent {
                    element: self.owner.clone(),
                });
            }
            Location::Primitive(_) | Location::Attribute | Location::Deferred => {
                return Err(ResolveError::NoChildren {
                    element: self.owner.clone(),
                    name: String::from(EXTENSION_ELEMENT),
                });
            }
        };
        self.owner.clone_from(&path);
        self.location = self.landing(field.kind)?;
        self.moves.push(Move::Extension {
            field: Field {
                path,
                key: String::from(field.name),
                kind: field.kind,
                min: field.min,
                max: field.max,
                types: field.types,
            },
            url: String::from(url),
        });
        Ok(())
    }

    /// Hands the reference the walk stands on to the engine.
    fn resolve_step(&mut self, rest: &[Step]) -> Result<(), ResolveError> {
        let referential = match &self.location {
            Location::Complex(schema) => schema.name == "Reference",
            Location::Primitive(_) => match self.moves.last() {
                Some(Move::Member(field)) => {
                    field.types.iter().any(|code| REFERENTIAL.contains(code))
                }
                _ => false,
            },
            Location::Attribute
            | Location::PrimitiveElement
            | Location::Choice(_)
            | Location::Resource
            | Location::Deferred => false,
        };
        if !referential {
            return Err(ResolveError::NotAReference {
                element: self.owner.clone(),
            });
        }
        let (expected, rest) = match rest.split_first() {
            Some((Step::Type { name, .. }, tail)) if self.table.is_resource(name) => {
                (Some(name.clone()), tail)
            }
            _ => (None, rest),
        };
        self.defer(expected, rest);
        Ok(())
    }

    /// Records the deferred resolution and what remains of the path.
    fn defer(&mut self, expected: Option<String>, rest: &[Step]) {
        self.location = Location::Deferred;
        self.moves.push(Move::Resolve {
            expected,
            continuation: rest.to_vec(),
        });
    }

    /// Records an index filter, which needs a repeating element.
    fn index(&mut self, position: usize) -> Result<(), ResolveError> {
        let repeats = match self.moves.last() {
            Some(Move::Member(field) | Move::Choice { field, .. }) => field.repeats(),
            Some(Move::Extension { .. }) => true,
            _ => false,
        };
        if !repeats {
            return Err(ResolveError::NotRepeating {
                element: self.owner.clone(),
                index: position,
            });
        }
        self.moves.push(Move::Index(position));
        Ok(())
    }

    /// Where a step lands, given the kind of the element it took.
    fn landing(&self, kind: Kind) -> Result<Location, ResolveError> {
        Ok(match kind {
            Kind::Attribute => Location::Attribute,
            Kind::Primitive(value) => Location::Primitive(value),
            Kind::Xhtml => Location::Primitive(ValueKind::Text),
            Kind::Resource => Location::Resource,
            Kind::Choice(_) => Location::Choice(self.owner.clone()),
            Kind::Complex(name) => {
                let schema =
                    self.table
                        .type_named(name)
                        .ok_or_else(|| ResolveError::UnknownElement {
                            owner: self.owner.clone(),
                            name: String::from(name),
                        })?;
                Location::Complex(schema)
            }
        })
    }
}

/// What [`named`] selects: the table entry, the kind and JSON key it lands on,
/// and the type codes the landing element admits.
type Named = (&'static FieldSchema, Kind, String, &'static [&'static str]);

/// The element `name` selects in `schema`, with the kind, the JSON key and
/// the type codes it lands on.
///
/// A name is an element name, or a choice element's name with the suffix of
/// one alternative, which is how the JSON representation spells a resolved
/// choice (<https://hl7.org/fhir/R4/json.html>).
fn named(schema: &'static TypeSchema, name: &str) -> Option<Named> {
    if let Some(field) = schema.fields.iter().find(|field| field.name == name) {
        return Some((field, field.kind, String::from(name), field.types));
    }
    schema.fields.iter().find_map(|field| {
        let Kind::Choice(variants) = field.kind else {
            return None;
        };
        let suffix = name.strip_prefix(field.name)?;
        variants
            .iter()
            .find(|(candidate, _)| *candidate == suffix)
            .map(|(_, kind)| {
                (
                    field,
                    *kind,
                    String::from(name),
                    alternative(field.types, suffix),
                )
            })
    })
}

/// The type codes of the choice alternative `suffix` names, out of the
/// choice's `types`.
fn alternative(types: &'static [&'static str], suffix: &str) -> &'static [&'static str] {
    // NOTE: a chosen alternative has the one type its suffix names
    // (<https://hl7.org/fhir/R4/formats.html#choice>), so the field keeps that code alone.
    types
        .iter()
        .position(|code| code.eq_ignore_ascii_case(suffix))
        .and_then(|at| types.get(at))
        .map_or(types, core::slice::from_ref)
}
