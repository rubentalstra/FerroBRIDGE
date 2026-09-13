// SPDX-FileCopyrightText: Ruben Talstra
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

use fhir_types::xml::FieldSchema;
use fhir_types::xml::Kind;
use fhir_types::xml::Schemas;
use fhir_types::xml::TypeSchema;
use fhir_types::xml::ValueKind;

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
        self.resources.binary_search(&name).is_ok()
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

/// The primitive type codes `resolve()` accepts.
///
/// `resolve()` reads "a string that is a uri (or canonical or url)"
/// (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions).
const REFERENTIAL: [&str; 3] = ["uri", "canonical", "url"];

/// The `id` every element carries, for the sibling object of a primitive.
///
/// A primitive's `id` and `extension` live in the member named with a leading
/// underscore (<https://hl7.org/fhir/R4/json.html>), and both come from
/// `Element` itself (<https://hl7.org/fhir/R4/element.html>).
const ELEMENT_ID: (&str, Kind, u32, Option<u32>, &[&str]) =
    ("id", Kind::Attribute, 0, Some(1), &["string"]);

/// The `extension` every element carries.
const ELEMENT_EXTENSION: (&str, Kind, u32, Option<u32>, &[&str]) = (
    "extension",
    Kind::Complex("Extension"),
    0,
    None,
    &["Extension"],
);

/// The resolver's state as it walks the steps.
struct Walk<'table, T: Table + ?Sized> {
    table: &'table T,
    location: Location,
    owner: String,
    moves: Vec<Move>,
}

impl<T: Table + ?Sized> Walk<'_, T> {
    /// Takes one step, answering whether the walk is finished.
    fn take(&mut self, step: &Step, rest: &[Step]) -> Result<bool, ResolveError> {
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
        let (field, kind, key) =
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
            types: field.types,
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
        let (element, kind, min, max, types) = match name {
            "id" => ELEMENT_ID,
            "extension" => ELEMENT_EXTENSION,
            _ => {
                return Err(ResolveError::NoChildren {
                    element: self.owner.clone(),
                    name: String::from(name),
                });
            }
        };
        let path = format!("{}.{element}", self.owner);
        self.owner.clone_from(&path);
        self.location = self.landing(kind)?;
        self.moves.push(Move::Member(Field {
            path,
            key: String::from(element),
            kind,
            min,
            max,
            types,
        }));
        Ok(())
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
        let chosen = Field {
            path: field.path.clone(),
            key: format!("{}{suffix}", field.key),
            kind: *kind,
            min: field.min,
            max: field.max,
            types: field.types,
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
            self.enter_primitive_element("extension")?;
        }
        let (element, kind, min, max, types) = ELEMENT_EXTENSION;
        let path = match &self.location {
            Location::Complex(schema) => {
                let field = named(schema, element).ok_or_else(|| ResolveError::UnknownElement {
                    owner: String::from(schema.path),
                    name: String::from(element),
                })?;
                String::from(field.0.path)
            }
            Location::PrimitiveElement => format!("{}.{element}", self.owner),
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
                    name: String::from(element),
                });
            }
        };
        self.owner.clone_from(&path);
        self.location = self.landing(kind)?;
        self.moves.push(Move::Extension {
            field: Field {
                path,
                key: String::from(element),
                kind,
                min,
                max,
                types,
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

/// The element `name` selects in `schema`, with the kind and the JSON key it
/// lands on.
///
/// A name is an element name, or a choice element's name with the suffix of
/// one alternative, which is how the JSON representation spells a resolved
/// choice (<https://hl7.org/fhir/R4/json.html>).
fn named(schema: &'static TypeSchema, name: &str) -> Option<(&'static FieldSchema, Kind, String)> {
    if let Some(field) = schema.fields.iter().find(|field| field.name == name) {
        return Some((field, field.kind, String::from(name)));
    }
    schema.fields.iter().find_map(|field| {
        let Kind::Choice(variants) = field.kind else {
            return None;
        };
        let suffix = name.strip_prefix(field.name)?;
        variants
            .iter()
            .find(|(candidate, _)| *candidate == suffix)
            .map(|(_, kind)| (field, *kind, String::from(name)))
    })
}

#[cfg(test)]
#[expect(clippy::panic_in_result_fn, reason = "test assertions")]
mod tests {
    use fhir_types::r4::schema::SCHEMAS;
    use fhir_types::xml::Kind;
    use fhir_types::xml::ValueKind;

    use crate::tree::element::Location;
    use crate::tree::element::Move;
    use crate::tree::element::Resolved;
    use crate::tree::element::resolve;
    use crate::tree::error::ParseError;
    use crate::tree::error::ResolveError;

    fn resolved(resource: &str, expression: &str) -> Result<Resolved, Box<dyn core::error::Error>> {
        let path = expression.parse()?;
        Ok(resolve(&SCHEMAS, resource, &path)?)
    }

    fn keys(resolved: &Resolved) -> Vec<String> {
        resolved
            .moves()
            .iter()
            .filter_map(|step| match step {
                Move::Member(field) | Move::Extension { field, .. } => {
                    Some(String::from(field.key()))
                }
                Move::Choice { field, .. } => Some(String::from(field.key())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_choice_resolves_to_the_suffixed_json_key() -> Result<(), Box<dyn core::error::Error>> {
        for expression in [
            "$resource.onset.ofType(Period)",
            "$resource.onset.as(Period)",
            "$resource.onsetPeriod",
        ] {
            let resolved = resolved("Condition", expression)?;
            assert_eq!(keys(&resolved), ["onsetPeriod"], "{expression}");
            assert_eq!(resolved.leaf(), "Condition.onset[x]", "{expression}");
        }
        Ok(())
    }

    #[test]
    fn a_type_the_choice_does_not_admit_names_the_element_and_the_alternatives()
    -> Result<(), ParseError> {
        let path = "$resource.onset.ofType(Quantity)".parse()?;
        let refused = resolve(&SCHEMAS, "Condition", &path);
        let Err(ResolveError::ChoiceType {
            element,
            requested,
            admitted,
        }) = refused
        else {
            panic!("a wrong choice type should have been refused")
        };
        assert_eq!(element, "Condition.onset[x]");
        assert_eq!(requested, "Quantity");
        assert_eq!(admitted, "DateTime, Age, Period, Range, String");
        Ok(())
    }

    #[test]
    fn an_unknown_element_names_the_type_that_defines_none() -> Result<(), ParseError> {
        let path = "$resource.onsettt".parse()?;
        let refused = resolve(&SCHEMAS, "Condition", &path);
        assert!(matches!(
            refused,
            Err(ResolveError::UnknownElement { ref owner, ref name })
                if owner == "Condition" && name == "onsettt"
        ));
        Ok(())
    }

    #[test]
    fn a_step_through_an_unresolved_choice_is_refused() -> Result<(), ParseError> {
        let path = "$resource.onset.start".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Condition", &path),
            Err(ResolveError::UnresolvedChoice { ref element }) if element == "Condition.onset[x]"
        ));
        Ok(())
    }

    #[test]
    fn a_primitive_carries_its_extension_in_the_underscore_sibling()
    -> Result<(), Box<dyn core::error::Error>> {
        let resolved = resolved(
            "Condition",
            "$resource.onset.ofType(Period).start.extension",
        )?;
        assert_eq!(keys(&resolved), ["onsetPeriod", "_start", "extension"]);
        assert_eq!(resolved.leaf(), "Period.start.extension");
        assert_eq!(
            resolved.location(),
            &Location::Complex(SCHEMAS.type_named("Extension").ok_or("no Extension type")?)
        );
        Ok(())
    }

    #[test]
    fn the_extension_shortcut_reaches_the_same_element() -> Result<(), Box<dyn core::error::Error>>
    {
        let resolved = resolved(
            "Condition",
            "$resource.extension('http://hl7.org/fhir/StructureDefinition/condition-assertedDate')",
        )?;
        assert_eq!(keys(&resolved), ["extension"]);
        let Some(Move::Extension { url, .. }) = resolved.moves().last() else {
            panic!("the shortcut should have resolved to an extension move")
        };
        assert_eq!(
            url,
            "http://hl7.org/fhir/StructureDefinition/condition-assertedDate"
        );
        Ok(())
    }

    #[test]
    fn a_content_reference_lands_on_the_type_it_names() -> Result<(), Box<dyn core::error::Error>> {
        let resolved = resolved("Bundle", "$resource.entry.link.relation")?;
        assert_eq!(resolved.leaf(), "Bundle.link.relation");
        assert_eq!(resolved.location(), &Location::Primitive(ValueKind::Text));
        Ok(())
    }

    #[test]
    fn a_resource_type_on_a_reference_defers_to_the_engine()
    -> Result<(), Box<dyn core::error::Error>> {
        for expression in [
            "$resource.recorder.resolve().as(Practitioner).name",
            "$resource.recorder.as(Practitioner).name",
        ] {
            let resolved = resolved("Condition", expression)?;
            assert_eq!(resolved.location(), &Location::Deferred, "{expression}");
            let Some(Move::Resolve {
                expected,
                continuation,
            }) = resolved.moves().last()
            else {
                panic!("`{expression}` should have deferred")
            };
            assert_eq!(expected.as_deref(), Some("Practitioner"), "{expression}");
            assert_eq!(continuation.len(), 1, "{expression}");
        }
        Ok(())
    }

    #[test]
    fn a_type_filter_on_the_element_s_own_type_is_an_assertion()
    -> Result<(), Box<dyn core::error::Error>> {
        let resolved = resolved(
            "Condition",
            "$resource.encounter.ofType(Reference).identifier",
        )?;
        assert_eq!(keys(&resolved), ["encounter", "identifier"]);
        assert_eq!(resolved.leaf(), "Reference.identifier");
        Ok(())
    }

    #[test]
    fn a_type_filter_that_is_not_the_element_s_type_names_both() -> Result<(), ParseError> {
        let path = "$resource.code.ofType(Quantity)".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Condition", &path),
            Err(ResolveError::TypeAssertion { ref requested, ref actual, .. })
                if requested == "Quantity" && actual == "CodeableConcept"
        ));
        Ok(())
    }

    #[test]
    fn a_type_filter_on_a_primitive_reads_the_definition_s_own_type_code()
    -> Result<(), Box<dyn core::error::Error>> {
        let resolved = resolved("Condition", "$resource.recordedDate.as(DateTime)")?;
        assert_eq!(keys(&resolved), ["recordedDate"]);
        let refused = "$resource.recordedDate.as(Period)".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Condition", &refused),
            Err(ResolveError::TypeAssertion { ref actual, .. }) if actual == "dateTime"
        ));
        Ok(())
    }

    #[test]
    fn resolve_on_an_element_that_is_no_reference_is_refused() -> Result<(), ParseError> {
        let path = "$resource.recordedDate.resolve()".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Condition", &path),
            Err(ResolveError::NotAReference { ref element })
                if element == "Condition.recordedDate"
        ));
        Ok(())
    }

    #[test]
    fn resolve_on_a_canonical_primitive_defers() -> Result<(), Box<dyn core::error::Error>> {
        let resolved = resolved("Condition", "$resource.meta.profile.resolve()")?;
        assert_eq!(resolved.location(), &Location::Deferred);
        Ok(())
    }

    #[test]
    fn an_index_on_an_element_that_does_not_repeat_is_refused() -> Result<(), ParseError> {
        let path = "$resource.subject[0]".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Condition", &path),
            Err(ResolveError::NotRepeating { ref element, index: 0 })
                if element == "Condition.subject"
        ));
        Ok(())
    }

    #[test]
    fn an_unbound_expression_is_refused_before_the_table_is_touched() -> Result<(), ParseError> {
        let path = "code.coding".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Condition", &path),
            Err(ResolveError::NotAnchored { .. })
        ));
        Ok(())
    }

    #[test]
    fn an_unknown_resource_type_is_refused() -> Result<(), ParseError> {
        let path = "$resource.code".parse()?;
        assert!(matches!(
            resolve(&SCHEMAS, "Cndition", &path),
            Err(ResolveError::UnknownResource { ref name }) if name == "Cndition"
        ));
        Ok(())
    }

    #[test]
    fn a_repeating_element_is_read_from_the_table() -> Result<(), Box<dyn core::error::Error>> {
        let resolved = resolved("Condition", "$resource.category.coding.code")?;
        let Some(Move::Member(category)) = resolved.moves().first() else {
            panic!("the first move should be `category`")
        };
        assert!(category.repeats());
        assert_eq!(category.max(), None);
        assert_eq!(category.min(), 0);
        assert_eq!(category.kind(), Kind::Complex("CodeableConcept"));
        Ok(())
    }
}
