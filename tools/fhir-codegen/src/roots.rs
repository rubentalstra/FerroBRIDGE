// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The root sets: what the generator emits from a package, and under which
//! Cargo feature of the generated crate.
//!
//! Two root sets are declared. The terminology set is the resources of the
//! FHIR terminology module (<https://hl7.org/fhir/R4B/terminology-module.html>)
//! plus the infrastructure resources its operations exchange, and the
//! operations defined on `CodeSystem`, `ValueSet`, and `ConceptMap`. The
//! resource set is every concrete `kind: resource` `StructureDefinition` the
//! package defines. Each root set is declared here; the transitive closure of
//! the types those roots reference is the emitter's job.

use std::collections::BTreeMap;

use crate::fhir::{Derivation, OperationDefinition, StructureDefinition, StructureKind};
use crate::package::Package;

/// The resource types the terminology root set holds, by name.
pub const ROOT_RESOURCES: [&str; 8] = [
    "Bundle",
    "CapabilityStatement",
    "CodeSystem",
    "ConceptMap",
    "OperationOutcome",
    "Parameters",
    "TerminologyCapabilities",
    "ValueSet",
];

/// The resource types whose operations are terminology operations.
pub const OPERATION_RESOURCES: [&str; 3] = ["CodeSystem", "ConceptMap", "ValueSet"];

/// Which declared root set introduced a type.
///
/// The order is the inclusion order: the terminology set is a subset of the
/// resource set, so the smaller value is the narrower feature, and a type
/// reachable from both carries [`RootScope::Terminology`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RootScope {
    /// The terminology root set, behind the `terminology` feature.
    Terminology,
    /// Every concrete resource the package defines, behind the `resources` feature.
    Resources,
}

impl RootScope {
    /// The `fhir-types` feature that gates the scope.
    #[must_use]
    pub const fn feature(self) -> &'static str {
        match self {
            Self::Terminology => "terminology",
            Self::Resources => "resources",
        }
    }

    /// The `cfg` attribute line gating the scope, with a trailing newline.
    #[must_use]
    pub fn cfg(self) -> String {
        format!("#[cfg(feature = \"{}\")]\n", self.feature())
    }
}

/// A root resource the package does not define.
#[derive(Debug, thiserror::Error)]
#[error("the package defines no StructureDefinition named {name}")]
pub struct MissingRoot {
    /// The missing resource type name.
    pub name: String,
}

/// The root set selected from one package.
#[derive(Debug)]
pub struct RootSet<'a> {
    /// The root resource definitions, keyed by type name.
    pub resources: BTreeMap<&'a str, &'a StructureDefinition>,
    /// The terminology operations, keyed by canonical URL.
    pub operations: BTreeMap<&'a str, &'a OperationDefinition>,
    /// The scope that introduced each root resource.
    scopes: BTreeMap<&'a str, RootScope>,
}

impl<'a> RootSet<'a> {
    /// Selects the terminology root set and the terminology operations of `package`.
    ///
    /// # Errors
    ///
    /// Returns [`MissingRoot`] when the package defines no structure for one
    /// of [`ROOT_RESOURCES`].
    pub fn select(package: &'a Package) -> Result<Self, MissingRoot> {
        Self::select_scoped(package, RootScope::Terminology)
    }

    /// Selects the root set `scope` declares, and the terminology operations.
    ///
    /// [`RootScope::Resources`] widens the resource set to every concrete
    /// `kind: resource` structure the package defines; the operations are the
    /// terminology ones either way. An operation is a terminology operation
    /// when it applies to at least one resource type and every type it applies
    /// to is one of [`OPERATION_RESOURCES`].
    ///
    /// # Errors
    ///
    /// Returns [`MissingRoot`] when the package defines no structure for one
    /// of [`ROOT_RESOURCES`].
    pub fn select_scoped(package: &'a Package, scope: RootScope) -> Result<Self, MissingRoot> {
        let mut resources = BTreeMap::new();
        let mut scopes = BTreeMap::new();
        for name in ROOT_RESOURCES {
            let definition =
                package
                    .structure_definition_named(name)
                    .ok_or_else(|| MissingRoot {
                        name: name.to_owned(),
                    })?;
            resources.insert(name, definition);
            scopes.insert(name, RootScope::Terminology);
        }
        if scope == RootScope::Resources {
            for definition in package.structure_definitions().values() {
                if !is_concrete_resource(definition) {
                    continue;
                }
                let name = definition.name.as_str();
                if resources.contains_key(name) {
                    continue;
                }
                resources.insert(name, definition);
                scopes.insert(name, RootScope::Resources);
            }
        }
        let operations = package
            .operation_definitions()
            .iter()
            .filter(|(_, operation)| is_terminology_operation(operation))
            .map(|(url, operation)| (url.as_str(), operation))
            .collect();
        Ok(Self {
            resources,
            operations,
            scopes,
        })
    }

    /// The scope that introduced the root resource named `name`.
    #[must_use]
    pub fn scope(&self, name: &str) -> Option<RootScope> {
        self.scopes.get(name).copied()
    }

    /// The root resources `scope` includes, in name order.
    pub fn in_scope(&self, scope: RootScope) -> impl Iterator<Item = &'a StructureDefinition> {
        self.resources
            .iter()
            .filter(move |(name, _)| self.scopes.get(*name).copied() <= Some(scope))
            .map(|(_, definition)| *definition)
    }

    /// Whether any root resource comes from `scope`.
    #[must_use]
    pub fn holds(&self, scope: RootScope) -> bool {
        self.scopes.values().any(|held| *held == scope)
    }

    /// The terminology operation invoked as `$code` on `resource`, if any.
    #[must_use]
    pub fn operation(&self, resource: &str, code: &str) -> Option<&'a OperationDefinition> {
        self.operations.values().copied().find(|operation| {
            operation.code == code && operation.resource.iter().any(|r| r == resource)
        })
    }
}

/// Whether `definition` defines a resource type the wide root set holds: a
/// resource the specification instantiates, so neither an abstract base nor a
/// profile (<https://hl7.org/fhir/R4B/structuredefinition.html>).
fn is_concrete_resource(definition: &StructureDefinition) -> bool {
    definition.kind == StructureKind::Resource
        && !definition.is_abstract
        && definition.derivation == Some(Derivation::Specialization)
}

fn is_terminology_operation(operation: &OperationDefinition) -> bool {
    !operation.resource.is_empty()
        && operation
            .resource
            .iter()
            .all(|resource| OPERATION_RESOURCES.contains(&resource.as_str()))
}
