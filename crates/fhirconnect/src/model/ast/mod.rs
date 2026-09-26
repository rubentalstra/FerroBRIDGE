// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect file model.
//!
//! One Rust type per construct the FHIRconnect v1.0.0 specification defines
//! for a model, extension or context mapping file. Every node keeps the
//! [`Position`] of the YAML it was read from, so a refusal raised anywhere
//! downstream can name the place in the file that caused it.
//!
//! The nodes are plain records: the parser in [`crate::model::parse`] builds
//! them and nothing else mutates them, so their fields are public and every
//! invariant the grammar fixes is carried by the field's own type.
//!
//! Keyword values compare case-insensitively inside the documented set, and a
//! value outside the set is refused. YAML keys stay exact, because the
//! published schemas fix them with a JSON Schema `enum`, which matches
//! case-sensitively.

pub mod keyword;

use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::header::Header;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;

use crate::model::ast::keyword::ConditionOperator;
use crate::model::ast::keyword::DataType;
use crate::model::ast::keyword::Direction;
use crate::model::ast::keyword::ExtensionMethod;

/// One condition on a mapping method or on the file preprocessor.
///
/// Both `fhirCondition` and `openehrCondition` carry this shape
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`). The
/// singular `targetAttribute` and `criteria` keys and the plural
/// `targetAttributes` and `criterias` keys both lower into the vectors below,
/// so a file's spelling does not reach the interpreter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// Where the condition sits in the file.
    pub position: Position,
    /// The element returned once the filter has run.
    pub target_root: Located<String>,
    /// The paths, relative to `target_root`, the filter is applied to.
    pub target_attributes: Vec<Located<String>>,
    /// The test applied to the values found at the attributes.
    pub operator: Located<ConditionOperator>,
    /// The values the operator tests against.
    pub criteria: Vec<Located<String>>,
    /// Whether the condition identifies the element, as the published schemas
    /// spell the key.
    pub identifying: Option<Located<bool>>,
}

/// The `with` block: the two paths a mapping method maps against each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct With {
    /// Where the block sits in the file.
    pub position: Position,
    /// The `FHIRPath` side.
    pub fhir: Option<Located<String>>,
    /// The openEHR path side.
    pub openehr: Option<Located<String>>,
    /// The data type the pair is pinned to.
    pub data_type: Option<Located<DataType>>,
    /// The literal value the published model schema admits beside the paths.
    pub value: Option<Located<String>>,
}

/// One `path` and `value` pair inside a manual mapping entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPath {
    /// Where the pair sits in the file.
    pub position: Position,
    /// The path the value is written to.
    pub path: Located<String>,
    /// The value written at the path.
    pub value: Located<String>,
}

/// One entry of a `manual` mapping.
///
/// "Manual mappings are initialized using the `manual:` key which is followed
/// by a name … followed by a list of paths `- path:` and the value we map
/// `- value:`"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/manual.adoc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualEntry {
    /// Where the entry sits in the file.
    pub position: Position,
    /// The entry name.
    pub name: Located<String>,
    /// The FHIR-side paths and values.
    pub fhir: Vec<ManualPath>,
    /// The openEHR-side paths and values.
    pub openehr: Vec<ManualPath>,
    /// The FHIR-side condition selecting this entry.
    pub fhir_condition: Option<Condition>,
    /// The openEHR-side condition selecting this entry.
    pub openehr_condition: Option<Condition>,
    /// The literal value the published model schema admits on an entry.
    pub value: Option<Located<String>>,
    /// The direction the entry is restricted to.
    pub unidirectional: Option<Located<Direction>>,
}

/// A `link` mapping, which redirects part of a resource into another
/// composition and records an openEHR LINK to it.
///
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`,
/// §Linked mappings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMapping {
    /// Where the block sits in the file.
    pub position: Position,
    /// The `LINK.meaning` of the link that is created.
    pub meaning: Option<Located<String>>,
    /// The `LINK.type` of the link that is created.
    pub link_type: Option<Located<String>>,
}

/// A `reference` mapping, which initializes a further FHIR resource.
///
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/Reference.adoc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceMapping {
    /// Where the block sits in the file.
    pub position: Position,
    /// The FHIR resource type the reference initializes.
    pub resource_type: Located<String>,
    /// The mappings that run against the initialized resource.
    pub mappings: Vec<Mapping>,
}

/// A `followedBy` block: the mappings that run against the parent's path.
///
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/FollowedBy.adoc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedBy {
    /// Where the block sits in the file.
    pub position: Position,
    /// The child mappings, in document order.
    pub mappings: Vec<Mapping>,
}

/// One mapping method.
///
/// A mapping method opens with `- name:` and carries whichever of the concept
/// keys applies. The presence of a key is what selects the concept type: a
/// `mappingCode` marks a PROGRAMMED mapping, a `link` a LINKED one, a
/// `participationsFunction` a PARTICIPATION one, and `with.type: NONE` the
/// iteration marker
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "mappingCode is the key the FHIRconnect grammar spells"
)]
pub struct Mapping {
    /// Where the mapping sits in the file.
    pub position: Position,
    /// The mapping-method name, which extensions reference.
    pub name: Located<String>,
    /// The extension method, in an extension mapping file.
    pub extension: Option<Located<ExtensionMethod>>,
    /// The mapping method an `append` extension appends to, as a dotted path
    /// of mapping-method names.
    pub append_to: Option<Located<String>>,
    /// The pair of paths this method maps.
    pub with: Option<With>,
    /// The direction this method and its children are restricted to.
    pub unidirectional: Option<Located<Direction>>,
    /// The manual entries of a MANUAL mapping.
    pub manual: Vec<ManualEntry>,
    /// The FHIR-side condition guarding this method.
    pub fhir_condition: Option<Condition>,
    /// The openEHR-side condition guarding this method.
    pub openehr_condition: Option<Condition>,
    /// The child mappings that run after this one.
    pub followed_by: Option<FollowedBy>,
    /// The resource this method initializes, in a REFERENCE mapping.
    pub reference: Option<ReferenceMapping>,
    /// The `metadata.name` of the model mapping a SLOT mapping redirects to.
    pub slot_archetype: Option<Located<MappingName>>,
    /// The external function a PROGRAMMED mapping runs.
    pub mapping_code: Option<Located<String>>,
    /// The openEHR LINK a LINKED mapping creates.
    pub link: Option<LinkMapping>,
    /// The participation function a PARTICIPATION mapping carries.
    pub participations_function: Option<Located<String>>,
    /// The `ConceptMap.url` the codes of this method are translated through.
    pub conceptmap: Option<Located<String>>,
}

/// One side of a `hierarchy.split`.
///
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/HierarchyMappings.adoc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitTarget {
    /// Where the block sits in the file.
    pub position: Position,
    /// The kind of element to create for each occurrence.
    pub create: Option<Located<String>>,
    /// Where the element is created; absent when it is the archetype or the
    /// resource itself.
    pub path: Option<Located<String>>,
    /// The paths whose distinct values trigger a new element.
    pub unique: Vec<Located<String>>,
}

/// The `split` of a hierarchy mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchySplit {
    /// Where the block sits in the file.
    pub position: Position,
    /// What to create on the FHIR side.
    pub fhir: Option<SplitTarget>,
    /// What to create on the openEHR side.
    pub openehr: Option<SplitTarget>,
}

/// The one hierarchy mapping a file may carry in its preprocessor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hierarchy {
    /// Where the block sits in the file.
    pub position: Position,
    /// The pair of paths the split iterates.
    pub with: Option<With>,
    /// What each occurrence creates.
    pub split: Option<HierarchySplit>,
}

/// The preprocessor block of a model or extension mapping file.
///
/// A preprocessor condition decides whether the file runs at all
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §Conditions in the preprocessor), and the hierarchy mapping realigns the
/// two models before any mapping runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preprocessor {
    /// Where the block sits in the file.
    pub position: Position,
    /// The FHIR-side condition gating the file.
    pub fhir_condition: Option<Condition>,
    /// The openEHR-side condition gating the file.
    pub openehr_condition: Option<Condition>,
    /// The hierarchy realignment.
    pub hierarchy: Option<Hierarchy>,
}

/// The `spec` block of a model or extension mapping file.
///
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Spec. The
/// archetype and the revision are read by the shared header model, so they
/// are reached through [`ModelMappingFile::header`] rather than repeated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Where the block sits in the file.
    pub position: Position,
    /// The standard this file maps, which the published schema pins to `FHIR`.
    pub system: Located<String>,
    /// The release of that standard, which the published schema pins to `R4`.
    pub version: Located<String>,
    /// The `metadata.name` of the model mapping an extension file extends.
    pub extends: Option<Located<MappingName>>,
    /// The `StructureDefinition` the file records as its FHIR target.
    pub structure_definition: Option<Located<String>>,
    /// The `ConceptMap.url` every code in the file is translated through.
    pub conceptmap: Option<Located<String>>,
    /// The direction the whole file is restricted to.
    pub unidirectional: Option<Located<Direction>>,
}

/// One loaded model or extension mapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMappingFile {
    file: PathBuf,
    header: Header,
    spec: ModelSpec,
    preprocessor: Option<Preprocessor>,
    mappings: Vec<Mapping>,
}

impl ModelMappingFile {
    /// Builds a model mapping file from its parts.
    #[must_use]
    pub fn new(
        file: impl Into<PathBuf>,
        header: Header,
        spec: ModelSpec,
        preprocessor: Option<Preprocessor>,
        mappings: Vec<Mapping>,
    ) -> Self {
        Self {
            file: file.into(),
            header,
            spec,
            preprocessor,
            mappings,
        }
    }

    /// Returns the file the mapping was loaded from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Returns the shared header.
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// Returns the FHIRconnect half of the `spec` block.
    #[must_use]
    pub const fn spec(&self) -> &ModelSpec {
        &self.spec
    }

    /// Returns the preprocessor, when the file carries one.
    #[must_use]
    pub const fn preprocessor(&self) -> Option<&Preprocessor> {
        self.preprocessor.as_ref()
    }

    /// Returns the top-level mappings, in document order.
    #[must_use]
    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }
}

/// The profile a context mapping targets.
///
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/context-mappings.adoc`,
/// §profileUrl & templateId.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRef {
    /// Where the block sits in the file.
    pub position: Position,
    /// The canonical URL of the profile.
    pub url: Option<Located<String>>,
    /// The version of the profile.
    pub version: Option<Located<String>>,
}

/// The operational template a context mapping targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRef {
    /// Where the block sits in the file.
    pub position: Position,
    /// The exact template id.
    pub id: Option<Located<String>>,
    /// The version of the template.
    pub sem_ver: Option<Located<String>>,
}

/// The `context` block of a context mapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingContext {
    /// Where the block sits in the file.
    pub position: Position,
    /// The profile being mapped.
    pub profile: ProfileRef,
    /// The template being mapped.
    pub template: TemplateRef,
    /// The model mappings this context loads, by `metadata.name`.
    pub archetypes: Vec<Located<MappingName>>,
    /// The extension mappings this context loads, by `metadata.name`.
    pub extensions: Vec<Located<MappingName>>,
    /// The operational models this context triggers, by `metadata.name`.
    pub operational: Vec<Located<MappingName>>,
    /// The mapping the engine starts at, by `metadata.name`.
    pub start: Located<MappingName>,
}

/// One loaded context mapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMappingFile {
    file: PathBuf,
    header: Header,
    context: MappingContext,
}

impl ContextMappingFile {
    /// Builds a context mapping file from its parts.
    #[must_use]
    pub fn new(file: impl Into<PathBuf>, header: Header, context: MappingContext) -> Self {
        Self {
            file: file.into(),
            header,
            context,
        }
    }

    /// Returns the file the mapping was loaded from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Returns the shared header.
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// Returns the `context` block.
    #[must_use]
    pub const fn context(&self) -> &MappingContext {
        &self.context
    }
}

/// Walks a mapping and every mapping nested under it, parents first.
///
/// The nesting a mapping can carry is `followedBy.mappings` and
/// `reference.mappings`, so both are walked in document order.
#[must_use]
pub fn walk(mappings: &[Mapping]) -> Vec<&Mapping> {
    let mut found = Vec::new();
    let mut stack: Vec<&Mapping> = mappings.iter().rev().collect();
    while let Some(mapping) = stack.pop() {
        found.push(mapping);
        let children = mapping
            .reference
            .as_ref()
            .map(|reference| reference.mappings.as_slice())
            .into_iter()
            .chain(
                mapping
                    .followed_by
                    .as_ref()
                    .map(|followed| followed.mappings.as_slice()),
            );
        for child in children.flatten().rev() {
            stack.push(child);
        }
    }
    found
}
