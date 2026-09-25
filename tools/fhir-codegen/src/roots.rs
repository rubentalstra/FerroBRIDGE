// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The root sets: what the generator emits from a package, and under which
//! Cargo feature of the generated crate.
//!
//! Two FHIR root sets are declared, and one HL7 v2 root set. The terminology
//! set is the resources of the FHIR terminology module
//! (<https://hl7.org/fhir/R4B/terminology-module.html>) plus the
//! infrastructure resources its operations exchange, and the operations
//! defined on `CodeSystem`, `ValueSet`, and `ConceptMap`. The resource set is
//! every concrete `kind: resource` `StructureDefinition` the package defines.
//! The v2 set is every message structure of the fetched v2 definitions
//! ([`V2RootSet`]). Each root set is declared here; the transitive closure of
//! the types those roots reference is the emitter's job.

use std::collections::BTreeMap;

use crate::fhir::{Derivation, OperationDefinition, StructureDefinition, StructureKind};
use crate::package::Package;
use crate::v2::corpus::{Corpus, Sourced};

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

/// The v2 definitions directory holding one file per segment.
// NOTE: HL7/v2ig input/sourceOfTruth segment/segments-subset and
// message_structures-subset repeat canonical URLs of the full directories, so
// the root set reads the full directories only.
pub const V2_SEGMENT_DIR: &str = "segment/segments";

/// The v2 definitions directory holding one file per message structure.
pub const V2_STRUCTURE_DIR: &str = "message-structure/message_structures";

/// The v2 definitions directories holding the data types a field names.
pub const V2_DATA_TYPE_DIRS: [&str; 2] = [
    "data-type/primitive/primitives",
    "data-type/complex/complex-data-types",
];

/// The base models the segments and message structures specialize, by file.
// NOTE: HL7/v2ig input/sourceOfTruth meta-resources/message--message.json is
// not valid JSON (line 35), so the loader reads only the two base files named here.
pub const V2_BASES: [&str; 2] = [
    "meta-resources/segment--segment.json",
    "meta-resources/message-structure--message-structure.json",
];

/// The base every v2 segment definition specializes.
pub const V2_SEGMENT_BASE: &str = "http://hl7.org/v2/StructureDefinition/Segment";

/// The base every v2 message structure specializes: the roots of the v2 set.
pub const V2_STRUCTURE_BASE: &str = "http://hl7.org/v2/StructureDefinition/MessageStructure";

/// The v2 root set: every message structure and every segment definition of
/// the fetched definitions.
///
/// The segments are roots of their own, so a segment no structure references
/// (the batch envelopes `BHS`, `BTS`, `FHS` and `FTS`, and `ADD`, `FAC`, `OVR`,
/// `PDC` and `PSH`) is emitted beside the ones the structures reach. A
/// field's data type is checked to exist and emitted by its code only.
#[derive(Debug)]
pub struct V2RootSet<'a> {
    /// The message structures, keyed by id (`ORU_R01-A`).
    pub structures: BTreeMap<&'a str, &'a Sourced>,
    /// The segment definitions, keyed by id (`OBX`).
    // NOTE: no specification governs this: our own design; a batch delivered over
    // MLLP carries the BHS/FHS envelopes no message structure names, so every segment is a root.
    pub segments: BTreeMap<&'a str, &'a Sourced>,
}

/// The one file in the segment directory that is no segment definition: the
/// open `Hxx` slot, which the message structures inline as a group.
pub const V2_SLOT_FILE: &str = "segment/segments/Hxx.json";

/// A definition in a root directory that specializes another base than the
/// directory holds.
#[derive(Debug, thiserror::Error)]
#[error("{file} specializes {base:?}, not {expected}")]
pub struct NotAStructure {
    /// The file, relative to the definitions tree.
    pub file: String,
    /// The base it names.
    pub base: Option<String>,
    /// The base the directory's definitions specialize.
    pub expected: &'static str,
}

impl<'a> V2RootSet<'a> {
    /// Selects every message structure and every segment definition of `corpus`.
    ///
    /// # Errors
    ///
    /// Returns [`NotAStructure`] for a definition in the structure directory
    /// that specializes a base other than [`V2_STRUCTURE_BASE`], or one in the
    /// segment directory, [`V2_SLOT_FILE`] aside, that specializes a base
    /// other than [`V2_SEGMENT_BASE`].
    pub fn select(corpus: &'a Corpus) -> Result<Self, NotAStructure> {
        let structures = roots(corpus.structures().values(), V2_STRUCTURE_BASE)?;
        let segments = roots(
            corpus
                .segments()
                .values()
                .filter(|sourced| sourced.file != V2_SLOT_FILE),
            V2_SEGMENT_BASE,
        )?;
        Ok(Self {
            structures,
            segments,
        })
    }
}

/// The definitions of `sourced`, keyed by id, each checked to specialize `base`.
fn roots<'a>(
    sourced: impl Iterator<Item = &'a Sourced>,
    base: &'static str,
) -> Result<BTreeMap<&'a str, &'a Sourced>, NotAStructure> {
    let mut out = BTreeMap::new();
    for entry in sourced {
        let named = entry.definition.base_definition.as_deref();
        if named != Some(base) {
            return Err(NotAStructure {
                file: entry.file.clone(),
                base: named.map(str::to_owned),
                expected: base,
            });
        }
        out.insert(entry.definition.id.as_str(), entry);
    }
    Ok(out)
}
