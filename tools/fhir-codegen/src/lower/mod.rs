// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering resolved structures to the type definitions the renderer emits.
//!
//! Every structure in the closure becomes a struct; a backbone element
//! becomes a struct named by its path; a choice element (`value[x]`,
//! <https://hl7.org/fhir/R4B/formats.html#choice>) becomes an enum with one
//! variant per allowed type; a content reference points at the struct of the
//! element it names. Cardinality maps to `T`, `Option<T>`, and `Vec<T>`
//! (<https://hl7.org/fhir/R4B/conformance-rules.html#cardinality>), and an
//! `Option` or direct field that closes a type cycle is boxed.

mod cycle;
mod structure;

use std::collections::{BTreeMap, BTreeSet};

use crate::closure::{ELEMENT_TYPE, TypeClosure};
use crate::fhir::StructureKind;
use crate::lower::cycle::strongly_connected_components;
use crate::lower::structure::box_cyclic_fields;
use crate::lower::structure::box_cyclic_variants;
use crate::lower::structure::check_content_references;
use crate::lower::structure::contained_types;
use crate::lower::structure::lower_element_table_entry;
use crate::lower::structure::lower_struct;
use crate::naming::{module_name, type_name};
use crate::roots::RootScope;
use crate::snapshot::Max;

/// The module holding every primitive type.
pub const PRIMITIVES_MODULE: &str = "primitives";

/// The module holding the `Resource` enum.
pub const RESOURCE_MODULE: &str = "resource";

/// The name of the enum over the root-set resources.
pub const RESOURCE_ENUM: &str = "Resource";

/// The name of the struct carrying a resource outside the root set.
pub const UNKNOWN_RESOURCE: &str = "UnknownResource";

/// A failure while lowering.
#[derive(Debug, thiserror::Error)]
pub enum LowerError {
    /// A non-choice element lists more than one type code.
    #[error("{path} lists {count} types but is not a choice element")]
    MultiTyped {
        /// The element path.
        path: String,
        /// The number of type codes.
        count: usize,
    },
    /// A choice element lists no types.
    #[error("{path} is a choice element with no types")]
    EmptyChoice {
        /// The element path.
        path: String,
    },
    /// Two structures lower to the same Rust type name.
    #[error("two types lower to the Rust name {name}: {first} and {second}")]
    NameCollision {
        /// The colliding name.
        name: String,
        /// The first origin.
        first: String,
        /// The second origin.
        second: String,
    },
    /// Two structures lower to the same module (file) name.
    #[error("{first} and {second} both lower to the module {module}")]
    ModuleCollision {
        /// The colliding module name.
        module: String,
        /// The first type name.
        first: String,
        /// The second type name.
        second: String,
    },
    /// A `contentReference` names an element the emitted closure does not hold.
    #[error("{path} references {reference}, which the emitted closure does not hold")]
    UnresolvedContentReference {
        /// The referencing element path.
        path: String,
        /// The reference, as the element spells it.
        reference: String,
    },
    /// A primitive's lexical form is no regular expression the crate compiles.
    #[error("the lexical form of {path} does not compile: {reason}")]
    InvalidLexicalForm {
        /// The value element's path.
        path: String,
        /// The compiler's reason.
        reason: String,
    },
    /// A `contentReference` names an element behind a narrower feature.
    #[error(
        "{path} is emitted behind the {feature} feature and references {reference}, which is emitted behind {target_feature}"
    )]
    ContentReferenceOutOfScope {
        /// The referencing element path.
        path: String,
        /// The reference, as the element spells it.
        reference: String,
        /// The feature gating the referencing element.
        feature: &'static str,
        /// The feature gating the referenced element.
        target_feature: &'static str,
    },
}

/// How many values a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// Exactly one (`T`).
    One,
    /// Zero or one (`Option<T>`).
    Optional,
    /// Any number (`Vec<T>`).
    Many,
}

/// A value that is not a FHIR type: the `FHIRPath` system types primitives
/// carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    /// `bool`.
    Bool,
    /// `i32`.
    I32,
    /// `u32`.
    U32,
    /// `i64`.
    I64,
    /// `String`; decimals and dates keep their lexical form.
    Str,
}

/// What a field or variant points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A type in this model, by Rust name.
    Named(String),
    /// A scalar value.
    Inline(Scalar),
}

/// A field's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldType {
    /// The cardinality.
    pub card: Cardinality,
    /// The target.
    pub target: Target,
    /// Whether the value is boxed to break a type cycle.
    pub boxed: bool,
}

/// Documentation carried from the definition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Docs {
    /// The one-line summary.
    pub short: Option<String>,
    /// The formal definition.
    pub definition: Option<String>,
}

/// A struct field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The Rust field name.
    pub name: String,
    /// The FHIR element name (the last path segment, with `[x]` kept).
    pub fhir_name: String,
    /// The element path.
    pub path: String,
    /// Documentation.
    pub docs: Docs,
    /// The type.
    pub ty: FieldType,
    /// The minimum cardinality the definition states.
    pub min: u32,
    /// The maximum cardinality the definition states.
    pub max: Max,
    /// The type codes the definition lists, in its order; a choice lists its
    /// alternatives and a content reference lists none.
    pub types: Vec<String>,
    /// The element path a `contentReference` names, without the leading `#`.
    pub content_reference: Option<String>,
    /// The `isSummary` flag of the definition.
    pub is_summary: bool,
    /// The `isModifier` flag of the definition.
    pub is_modifier: bool,
}

/// A choice enum variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The Rust variant name.
    pub name: String,
    /// The FHIR type code.
    pub code: String,
    /// The type held.
    pub target: Target,
    /// Whether the value is boxed to break a type cycle.
    pub boxed: bool,
}

/// The shape of a Rust type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// A struct.
    Struct {
        /// The fields, in snapshot order.
        fields: Vec<Field>,
    },
    /// A choice enum.
    Choice {
        /// The FHIR element the choice belongs to.
        element_path: String,
        /// The variants, in the order the definition lists the types.
        variants: Vec<Variant>,
    },
    /// The enum over the root-set resources.
    ResourceEnum {
        /// The resource type names, in name order.
        resources: Vec<String>,
    },
    /// The struct carrying a resource outside the root set.
    UnknownResource,
}

/// One type definition to emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    /// The Rust name.
    pub name: String,
    /// The FHIR path the type was lowered from: a structure's root element
    /// path, a backbone or choice element's path, or the type's own name for
    /// the two types the emitter owns.
    pub path: String,
    /// The module (file) the type lives in.
    pub module: String,
    /// Documentation.
    pub docs: Docs,
    /// The shape.
    pub kind: TypeKind,
    /// Whether the type is a FHIR primitive.
    pub is_primitive: bool,
    /// Whether the type is a root-set resource.
    pub is_resource: bool,
    /// Whether the type is emitted into the element table alone, with no Rust
    /// type of its own ([`crate::closure::ELEMENT_TYPE`]).
    pub table_only: bool,
    /// The Rust name of the type this one specializes (`baseDefinition`), for
    /// a primitive: `Code` and `Id` specialize `String`, `Canonical` and `Url`
    /// specialize `Uri` (<https://hl7.org/fhir/R5/datatypes.html#primitive>).
    pub base: Option<String>,
    /// The narrowest root set that reaches the type, so the feature it is
    /// gated behind.
    pub scope: RootScope,
    /// The lexical form a primitive's value keeps, as the `regex` extension of
    /// its `value` element states it
    /// (<https://hl7.org/fhir/R5/datatypes.html#primitive>).
    pub value_regex: Option<String>,
}

/// The lexical form anchored to the whole value.
///
/// The specification writes each form unanchored and asks the reader to add
/// the anchors its regex engine spells
/// (<https://hl7.org/fhir/R5/datatypes.html#primitive>); the group keeps an
/// alternation inside the form from swallowing them.
#[must_use]
pub fn anchored_lexical_form(pattern: &str) -> String {
    format!("^(?:{pattern})$")
}

/// Compiles `pattern` the way the generated crate compiles it.
///
/// The forms are XML Schema patterns, where `\s` is the space, the tab, the
/// carriage return and the line feed alone
/// (<https://www.w3.org/TR/xmlschema-2/#regexs>), so Unicode mode stays off and
/// the match runs over the value's bytes. With it on, `\S` would exclude every
/// Unicode space and refuse values the packages themselves publish.
///
/// # Errors
///
/// Returns the compiler's error for a pattern it cannot build.
pub fn compile_lexical_form(pattern: &str) -> Result<regex::bytes::Regex, regex::Error> {
    regex::bytes::RegexBuilder::new(pattern)
        .unicode(false)
        .build()
}

/// Where a lowered structure lands in the generated module.
#[derive(Debug, Clone, Copy)]
struct Placement<'a> {
    /// The Rust type name.
    name: &'a str,
    /// The module (file) the type lives in.
    module: &'a str,
    /// The feature scope of the type.
    scope: RootScope,
    /// Whether the structure is a FHIR primitive.
    is_primitive: bool,
    /// Whether the structure is a root-set resource.
    is_resource: bool,
    /// Whether the structure is emitted into the element table alone.
    table_only: bool,
}

impl<'a> Placement<'a> {
    /// The placement of a backbone element nested in this one.
    fn nested(self, name: &'a str) -> Self {
        Self {
            name,
            module: self.module,
            scope: self.scope,
            is_primitive: false,
            is_resource: false,
            table_only: self.table_only,
        }
    }
}

/// The generated module for one FHIR version: every type it emits.
#[derive(Debug)]
pub struct VersionModule {
    /// The module name, for example `r4b`.
    pub name: String,
    /// The package the model was lowered from.
    pub package_name: String,
    /// The terminology operations the version declares, in module order.
    pub operations: Vec<crate::operations::OperationContract>,
    /// The package version.
    pub package_version: String,
    /// Every type, keyed by Rust name.
    pub types: BTreeMap<String, TypeDef>,
}

impl VersionModule {
    /// Lowers `closure` into a model.
    ///
    /// # Errors
    ///
    /// Returns [`LowerError`] for a non-choice element with several types, a
    /// choice with none, two structures lowering to one Rust name, or a
    /// `contentReference` the closure does not hold in a wide enough scope.
    pub fn lower(
        closure: &TypeClosure,
        version_module: &str,
        package_name: &str,
        package_version: &str,
    ) -> Result<Self, LowerError> {
        check_content_references(closure.structures(), |name| {
            closure.scope(name).unwrap_or(RootScope::Resources)
        })?;
        let mut model = Self {
            name: version_module.to_owned(),
            package_name: package_name.to_owned(),
            package_version: package_version.to_owned(),
            operations: Vec::new(),
            types: BTreeMap::new(),
        };
        // NOTE: the two emitter-owned modules are claimed first, so a FHIR
        // type whose module name would land in one is reported, never merged.
        let mut owners: BTreeMap<String, String> = BTreeMap::from([
            (
                PRIMITIVES_MODULE.to_owned(),
                String::from("(the primitives)"),
            ),
            (RESOURCE_MODULE.to_owned(), RESOURCE_ENUM.to_owned()),
            (
                module_name(ELEMENT_TYPE),
                String::from("(the element table)"),
            ),
        ]);
        lower_element_table_entry(&mut model, closure.element())?;
        for structure in closure.structures().values() {
            let is_primitive = structure.kind == StructureKind::PrimitiveType;
            let name = type_name(&structure.name);
            let module = if is_primitive {
                PRIMITIVES_MODULE.to_owned()
            } else {
                module_name(&name)
            };
            if !is_primitive && let Some(first) = owners.insert(module.clone(), name.clone()) {
                return Err(LowerError::ModuleCollision {
                    module,
                    first,
                    second: name,
                });
            }
            let root_path = structure
                .elements
                .first()
                .map_or_else(|| structure.name.clone(), |e| e.path.clone());
            let placement = Placement {
                name: &name,
                module: &module,
                scope: closure
                    .scope(&structure.name)
                    .unwrap_or(RootScope::Resources),
                is_primitive,
                is_resource: closure.roots().contains(&structure.name),
                table_only: false,
            };
            lower_struct(&mut model, structure, &root_path, placement)?;
        }
        let resources: Vec<String> = closure.roots().iter().map(|name| type_name(name)).collect();
        model.insert(TypeDef {
            name: RESOURCE_ENUM.to_owned(),
            path: RESOURCE_ENUM.to_owned(),
            module: RESOURCE_MODULE.to_owned(),
            docs: Docs {
                short: Some(String::from("A resource of the root set, or an unknown resource carried as JSON.")),
                definition: Some(
                    format!("The abstract Resource type (https://hl7.org/fhir/{}/resource.html) as the root set closes over it: one variant per root-set resource the enabled features select, and UnknownResource for any other resource type met inside a Bundle entry or a contained list.", version_module.to_uppercase()),
                ),
            },
            kind: TypeKind::ResourceEnum { resources },
            is_primitive: false,
            is_resource: false,
            table_only: false,
            base: None,
            scope: RootScope::Terminology,
            value_regex: None,
        }, "the Resource enum")?;
        model.insert(TypeDef {
            name: UNKNOWN_RESOURCE.to_owned(),
            path: UNKNOWN_RESOURCE.to_owned(),
            module: RESOURCE_MODULE.to_owned(),
            docs: Docs {
                short: Some(String::from("A resource outside the root set, kept as its JSON body.")),
                definition: Some(String::from(
                    "Carries the resourceType and the complete JSON object so a Bundle or a contained resource of a type outside the root set round-trips unchanged.",
                )),
            },
            kind: TypeKind::UnknownResource,
            is_primitive: false,
            is_resource: false,
            table_only: false,
            scope: RootScope::Terminology,
            base: None,
            value_regex: None,
        }, "the UnknownResource struct")?;
        model.box_cycles();
        model.box_wide_variants();
        Ok(model)
    }

    fn insert(&mut self, ty: TypeDef, origin: &str) -> Result<(), LowerError> {
        if let Some(existing) = self.types.get(&ty.name) {
            return Err(LowerError::NameCollision {
                name: ty.name.clone(),
                first: existing.module.clone(),
                second: origin.to_owned(),
            });
        }
        self.types.insert(ty.name.clone(), ty);
        Ok(())
    }

    /// The feature scope of `module`: the narrowest scope its types carry.
    ///
    /// A module holds one root structure with its backbone elements and choice
    /// enums, so the scopes agree; the primitives module is the exception, and
    /// a primitive only the wide root set reaches still compiles under the
    /// narrower feature.
    #[must_use]
    pub fn module_scope(&self, module: &str) -> RootScope {
        self.types
            .values()
            .filter(|ty| ty.module == module)
            .map(|ty| ty.scope)
            .min()
            .unwrap_or(RootScope::Terminology)
    }

    /// The modules of the model, in name order, each with its types in name order.
    ///
    /// A table-only type has no Rust type, so it appears in no module.
    #[must_use]
    pub fn modules(&self) -> BTreeMap<&str, Vec<&TypeDef>> {
        let mut modules: BTreeMap<&str, Vec<&TypeDef>> = BTreeMap::new();
        for ty in self.types.values().filter(|ty| !ty.table_only) {
            modules.entry(ty.module.as_str()).or_default().push(ty);
        }
        modules
    }

    /// Boxes every direct or optional edge that lies inside a type cycle.
    fn box_cycles(&mut self) {
        let sccs = strongly_connected_components(&self.edges());
        let component_of: BTreeMap<String, usize> = sccs
            .iter()
            .enumerate()
            .flat_map(|(index, component)| component.iter().map(move |name| (name.clone(), index)))
            .collect();
        for ty in self.types.values_mut() {
            let own = component_of.get(&ty.name).copied();
            match &mut ty.kind {
                TypeKind::Struct { fields } => box_cyclic_fields(fields, own, &component_of),
                TypeKind::Choice { variants, .. } => {
                    box_cyclic_variants(variants, own, &component_of);
                }
                TypeKind::ResourceEnum { .. } | TypeKind::UnknownResource => {}
            }
        }
    }

    /// Boxes every choice variant holding a complex type, so a choice enum is
    /// only as wide as the widest primitive it admits.
    ///
    /// A Rust enum is as large as its largest variant, so one rare wide type
    /// sets the size every value of the enum pays. `Parameters.value[x]` admits
    /// `Dosage`, which carries a whole `Timing`, and that made every
    /// `valueString` of an answer cost the same as a dosage schedule. Primitives
    /// stay inline: their width is bounded by the base element and they are what
    /// a terminology answer is nearly all of.
    fn box_wide_variants(&mut self) {
        let primitives: BTreeSet<String> = self
            .types
            .values()
            .filter(|ty| ty.is_primitive)
            .map(|ty| ty.name.clone())
            .collect();
        for ty in self.types.values_mut() {
            if let TypeKind::Choice { variants, .. } = &mut ty.kind {
                for variant in variants.iter_mut() {
                    if let Target::Named(target) = &variant.target
                        && !primitives.contains(target)
                    {
                        variant.boxed = true;
                    }
                }
            }
        }
    }

    /// The direct-containment graph: edges for `T` and `Option<T>` fields and
    /// for enum variants; `Vec<T>` already breaks recursion.
    fn edges(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for ty in self.types.values() {
            let out = edges.entry(ty.name.clone()).or_default();
            contained_types(&ty.kind, out);
        }
        edges
    }
}

/// The Rust scalar for a FHIR primitive's value, by primitive name.
///
/// The package types positiveInt and unsignedInt values as System.String;
/// the FHIR JSON representation carries them as numbers, so the scalar follows
/// the primitive's definition (<https://hl7.org/fhir/R4B/datatypes.html#primitive>).
/// Decimals and the date and time primitives keep their lexical form so
/// precision and partial dates survive.
pub(crate) fn scalar_for(code: &str) -> Scalar {
    match code {
        "boolean" => Scalar::Bool,
        "integer" => Scalar::I32,
        "positiveInt" | "unsignedInt" => Scalar::U32,
        "integer64" => Scalar::I64,
        _ => Scalar::Str,
    }
}
