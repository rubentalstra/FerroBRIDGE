// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The root-set closure: every type the root resources reach.
//!
//! Starting from the root resources, the closure follows every element type
//! and every content reference to the `StructureDefinition` that defines it,
//! transitively, so the emitted module holds the complete set of datatypes
//! and primitives the root set can carry, and nothing else (`codegen.md`:
//! complete within a declared closure). Each type also records the narrowest
//! root set that reaches it, which is the feature the emitted type is gated
//! behind.

use std::collections::{BTreeMap, BTreeSet};

use crate::fhir::{Derivation, StructureDefinition, StructureKind};
use crate::package::Package;
use crate::roots::{RootScope, RootSet};
use crate::snapshot::{ElementShape, ResolveError, ResolvedElement, ResolvedStructure};

/// Type codes that name a structural base rather than a datatype to emit.
///
/// `Element` and `BackboneElement` mark nested structures, which are emitted
/// as part of their parent; `Resource` is the abstract resource type, emitted
/// as the `Resource` enum over the root set.
pub const STRUCTURAL_TYPES: [&str; 3] = ["BackboneElement", "Element", "Resource"];

/// The base type of every element, resolved for the element table alone.
///
/// The FHIR JSON representation carries a primitive's `id` and `extension` in
/// a sibling member named with a leading underscore
/// (<https://hl7.org/fhir/R4/json.html>), and that member's shape is `Element`
/// (<https://hl7.org/fhir/R4/element.html>). No emitted field targets the
/// type, so it stays a structural type for the closure and joins the model as
/// a table-only entry ([`crate::lower::TypeDef::table_only`]).
pub const ELEMENT_TYPE: &str = "Element";

/// A failure while computing the closure.
#[derive(Debug, thiserror::Error)]
pub enum ClosureError {
    /// An element names a type the package does not define.
    #[error("{path} has type {code}, which the package does not define")]
    UnknownType {
        /// The element path.
        path: String,
        /// The type code.
        code: String,
    },
    /// An element names a type whose definition is a profile, not a type.
    #[error("{path} has type {code}, whose definition is a constraint profile, not a type")]
    ProfileAsType {
        /// The element path.
        path: String,
        /// The type code.
        code: String,
    },
    /// A snapshot failed to resolve.
    #[error(transparent)]
    Resolve(#[from] ResolveError),
}

/// The resolved structures of the root set and everything they reference.
#[derive(Debug)]
pub struct TypeClosure {
    structures: BTreeMap<String, ResolvedStructure>,
    element: ResolvedStructure,
    roots: BTreeSet<String>,
    scopes: BTreeMap<String, RootScope>,
}

impl TypeClosure {
    /// Computes the closure of `roots` over `package`.
    ///
    /// Every structure records the narrowest scope of `roots` that reaches it,
    /// so a datatype the terminology set already carries stays terminology
    /// when the wide resource set reaches it too.
    ///
    /// # Errors
    ///
    /// Returns [`ClosureError`] when an element names an undefined type or a
    /// profile, or when a snapshot does not resolve.
    pub fn compute(package: &Package, roots: &RootSet<'_>) -> Result<Self, ClosureError> {
        let structures = walk(package, roots.resources.values().copied())?;
        let terminology: BTreeSet<String> = if roots.holds(RootScope::Resources) {
            walk(package, roots.in_scope(RootScope::Terminology))?
                .into_keys()
                .collect()
        } else {
            structures.keys().cloned().collect()
        };
        let scopes = structures
            .keys()
            .map(|name| {
                let scope = if terminology.contains(name) {
                    RootScope::Terminology
                } else {
                    RootScope::Resources
                };
                (name.clone(), scope)
            })
            .collect();
        Ok(Self {
            structures,
            element: resolve_named(package, ELEMENT_TYPE, "(the element table)")?,
            roots: roots
                .resources
                .values()
                .map(|definition| definition.name.clone())
                .collect(),
            scopes,
        })
    }

    /// Every structure in the closure, keyed by type name, in name order.
    #[must_use]
    pub fn structures(&self) -> &BTreeMap<String, ResolvedStructure> {
        &self.structures
    }

    /// The base `Element` structure, for the table-only entry.
    #[must_use]
    pub fn element(&self) -> &ResolvedStructure {
        &self.element
    }

    /// The names of the root resources.
    #[must_use]
    pub fn roots(&self) -> &BTreeSet<String> {
        &self.roots
    }

    /// The narrowest root set that reaches the structure named `name`.
    #[must_use]
    pub fn scope(&self, name: &str) -> Option<RootScope> {
        self.scopes.get(name).copied()
    }

    /// The structures of one kind, in name order.
    pub fn of_kind(&self, kind: StructureKind) -> impl Iterator<Item = &ResolvedStructure> {
        self.structures
            .values()
            .filter(move |structure| structure.kind == kind)
    }
}

/// Resolves `roots` and everything they reference, transitively.
fn walk<'a>(
    package: &Package,
    roots: impl Iterator<Item = &'a StructureDefinition>,
) -> Result<BTreeMap<String, ResolvedStructure>, ClosureError> {
    let mut structures = BTreeMap::new();
    let mut pending: Vec<(String, String)> = roots
        .map(|definition| (definition.name.clone(), String::from("(root)")))
        .collect();
    while let Some((code, referrer)) = pending.pop() {
        if structures.contains_key(&code) || STRUCTURAL_TYPES.contains(&code.as_str()) {
            continue;
        }
        let resolved = resolve_named(package, &code, &referrer)?;
        if resolved.kind != StructureKind::PrimitiveType {
            for element in &resolved.elements {
                referenced(package, element, &mut pending);
            }
        }
        structures.insert(code, resolved);
    }
    Ok(structures)
}

/// The structure `code` names, refused when it is undefined or is a profile.
///
/// `referrer` is the element path that named it, so a failure says where the
/// reference came from.
fn resolve_named(
    package: &Package,
    code: &str,
    referrer: &str,
) -> Result<ResolvedStructure, ClosureError> {
    let definition =
        package
            .structure_definition_named(code)
            .ok_or_else(|| ClosureError::UnknownType {
                path: referrer.to_owned(),
                code: code.to_owned(),
            })?;
    if definition.derivation == Some(Derivation::Constraint) {
        return Err(ClosureError::ProfileAsType {
            path: referrer.to_owned(),
            code: code.to_owned(),
        });
    }
    Ok(ResolvedStructure::resolve(definition)?)
}

/// Queues every type `element` names that the closure has still to resolve.
///
/// A `FHIRPath` type is a primitive the generator maps itself, and a content
/// reference names its target structure by URL.
fn referenced(package: &Package, element: &ResolvedElement, pending: &mut Vec<(String, String)>) {
    match &element.shape {
        ElementShape::Typed(types) | ElementShape::Choice(types) => {
            for type_ref in types {
                if type_ref.fhirpath_type.is_none() {
                    pending.push((type_ref.code.clone(), element.path.clone()));
                }
            }
        }
        ElementShape::ContentReference {
            structure: Some(url),
            ..
        } => {
            if let Some(target) = package.structure_definitions().get(url) {
                pending.push((target.name.clone(), element.path.clone()));
            }
        }
        ElementShape::ContentReference { .. } | ElementShape::Root => {}
    }
}
