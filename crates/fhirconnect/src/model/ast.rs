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

use core::fmt;
use core::str::FromStr;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::header::Header;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;

/// Why a keyword value was refused.
///
/// The message lists every value the keyword admits, so a mapping author sees
/// the whole set rather than only that the value was wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not one of {}", render_admitted(admitted))]
pub struct KeywordError {
    /// The refused value, as the file writes it.
    pub value: String,
    /// Every value the keyword admits.
    pub admitted: &'static [&'static str],
}

/// Renders an admitted set as a comma-separated backticked list.
fn render_admitted(admitted: &[&str]) -> String {
    admitted
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// The direction a `unidirectional` key pins a file or a mapping to.
///
/// The two spellings are fixed by the header page ("`unidirectional` can be
/// set in the `spec` with `\"openehr->fhir\"` or `\"fhir->openehr\"`",
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Direction)
/// and repeated for a mapping method in
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/body.adoc`, §Undirectional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// The file or mapping runs openEHR to FHIR only.
    OpenehrToFhir,
    /// The file or mapping runs FHIR to openEHR only.
    FhirToOpenehr,
}

impl Direction {
    /// Every spelling the specification writes.
    pub const ADMITTED: &'static [&'static str] = &["openehr->fhir", "fhir->openehr"];

    /// Returns the spelling the specification writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenehrToFhir => "openehr->fhir",
            Self::FhirToOpenehr => "fhir->openehr",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Direction {
    type Err = KeywordError;

    /// Parses the value case-insensitively inside the documented set.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the two spellings.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case(Self::OpenehrToFhir.as_str()) {
            Ok(Self::OpenehrToFhir)
        } else if value.eq_ignore_ascii_case(Self::FhirToOpenehr.as_str()) {
            Ok(Self::FhirToOpenehr)
        } else {
            Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            })
        }
    }
}

/// How an extension mapping interacts with the model mapping it extends.
///
/// The three methods are fixed by
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`
/// and by the `properties.extension.enum` of the published model schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExtensionMethod {
    /// Adds a mapping at the bottom of the model mapping.
    Add,
    /// Appends a `followedBy` to the mapping method `appendTo` names.
    Append,
    /// Replaces the mapping method of the same name.
    Overwrite,
}

impl ExtensionMethod {
    /// Every spelling the schema enum admits.
    pub const ADMITTED: &'static [&'static str] = &["add", "append", "overwrite"];

    /// Returns the spelling the schema enum fixes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Append => "append",
            Self::Overwrite => "overwrite",
        }
    }
}

impl fmt::Display for ExtensionMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExtensionMethod {
    type Err = KeywordError;

    /// Parses the value case-exactly.
    ///
    /// The published model schema fixes the value with a JSON Schema `enum`
    /// (`properties.extension.enum`), and a JSON Schema enum matches
    /// case-sensitively, so `Add` is not this value.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the three methods.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "add" => Ok(Self::Add),
            "append" => Ok(Self::Append),
            "overwrite" => Ok(Self::Overwrite),
            _ => Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            }),
        }
    }
}

/// The data type a `with` block pins the pair of paths to.
///
/// The set is the `enum` of the published model schema
/// (`#/$defs/mapping/properties/type/enum`); the deprecated static-typing
/// table in
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/data-mappings.adoc`
/// names the same identifiers, and `NONE` is the iteration marker described in
/// `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataType {
    /// No transformation; the mapping iterates so its children can map.
    None,
    /// A FHIR Quantity.
    Quantity,
    /// A FHIR dateTime.
    DateTime,
    /// A FHIR `CodeableConcept`.
    CodeableConcept,
    /// A FHIR Coding.
    Coding,
    /// A FHIR string.
    String,
    /// A FHIR Dosage.
    Dosage,
    /// A FHIR id.
    Id,
    /// A FHIR Identifier.
    Identifier,
    /// A FHIR Ratio, against an openEHR `DV_PROPORTION`.
    Proportion,
}

impl DataType {
    /// Every spelling the schema enum admits.
    pub const ADMITTED: &'static [&'static str] = &[
        "NONE",
        "QUANTITY",
        "DATETIME",
        "CODEABLECONCEPT",
        "CODING",
        "STRING",
        "DOSAGE",
        "ID",
        "IDENTIFIER",
        "PROPORTION",
    ];

    /// Returns the spelling the schema enum fixes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Quantity => "QUANTITY",
            Self::DateTime => "DATETIME",
            Self::CodeableConcept => "CODEABLECONCEPT",
            Self::Coding => "CODING",
            Self::String => "STRING",
            Self::Dosage => "DOSAGE",
            Self::Id => "ID",
            Self::Identifier => "IDENTIFIER",
            Self::Proportion => "PROPORTION",
        }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DataType {
    type Err = KeywordError;

    /// Parses the value case-exactly, as the schema `enum` fixes it.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the ten data types.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "NONE" => Ok(Self::None),
            "QUANTITY" => Ok(Self::Quantity),
            "DATETIME" => Ok(Self::DateTime),
            "CODEABLECONCEPT" => Ok(Self::CodeableConcept),
            "CODING" => Ok(Self::Coding),
            "STRING" => Ok(Self::String),
            "DOSAGE" => Ok(Self::Dosage),
            "ID" => Ok(Self::Id),
            "IDENTIFIER" => Ok(Self::Identifier),
            "PROPORTION" => Ok(Self::Proportion),
            _ => Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            }),
        }
    }
}

/// The test a condition applies to the values under its `targetAttributes`.
///
/// The five operators and their combining logic come from the operator table
/// in `docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConditionOperator {
    /// The path holds one of the criteria; the criteria combine with OR.
    OneOf,
    /// The path holds none of the criteria; the criteria combine with AND.
    NotOf,
    /// The path is empty; it takes no criteria.
    Empty,
    /// The path is not empty; it takes no criteria.
    NotEmpty,
    /// The element at the path is of the type named in the criteria.
    Type,
}

impl ConditionOperator {
    /// Every spelling the operator table writes.
    pub const ADMITTED: &'static [&'static str] =
        &["one of", "not of", "empty", "not empty", "type"];

    /// Returns the spelling the operator table writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneOf => "one of",
            Self::NotOf => "not of",
            Self::Empty => "empty",
            Self::NotEmpty => "not empty",
            Self::Type => "type",
        }
    }

    /// Whether the operator takes a `criteria`.
    ///
    /// "`criteria` is not required for `empty` or `not empty`"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
    /// §criteria), and both operator rows say the operator "does not include a
    /// `criteria`".
    #[must_use]
    pub const fn takes_criteria(self) -> bool {
        match self {
            Self::OneOf | Self::NotOf | Self::Type => true,
            Self::Empty | Self::NotEmpty => false,
        }
    }
}

impl fmt::Display for ConditionOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConditionOperator {
    type Err = KeywordError;

    /// Parses the value case-exactly, as the operator table spells it.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a value outside the five operators.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "one of" => Ok(Self::OneOf),
            "not of" => Ok(Self::NotOf),
            "empty" => Ok(Self::Empty),
            "not empty" => Ok(Self::NotEmpty),
            "type" => Ok(Self::Type),
            _ => Err(KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            }),
        }
    }
}

/// A `$name` variable at the head of a FHIRconnect path.
///
/// The seven variables are the rows of the variable table in
/// `docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Variable {
    /// The root of the FHIR resource.
    Resource,
    /// The parent FHIR path of the enclosing mapping step.
    FhirRoot,
    /// The root of the openEHR archetype.
    Archetype,
    /// The parent openEHR path of the enclosing mapping step.
    OpenehrRoot,
    /// The root of the enclosing openEHR composition.
    Composition,
    /// The marker for a path skipped because a reference resolves it.
    Reference,
    /// The values passed in on the REST call, in operational mappings.
    Context,
}

impl Variable {
    /// Every variable name the table lists, without the leading `$`.
    pub const ADMITTED: &'static [&'static str] = &[
        "resource",
        "fhirRoot",
        "archetype",
        "openehrRoot",
        "composition",
        "reference",
        "context",
    ];

    /// Returns the name the table writes, without the leading `$`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::FhirRoot => "fhirRoot",
            Self::Archetype => "archetype",
            Self::OpenehrRoot => "openehrRoot",
            Self::Composition => "composition",
            Self::Reference => "reference",
            Self::Context => "context",
        }
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.as_str())
    }
}

impl FromStr for Variable {
    type Err = KeywordError;

    /// Parses a variable name, with or without the leading `$`,
    /// case-insensitively inside the documented set.
    ///
    /// # Errors
    ///
    /// Returns [`KeywordError`] for a name outside the seven variables.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let name = value.strip_prefix('$').unwrap_or(value);
        let admitted = [
            Self::Resource,
            Self::FhirRoot,
            Self::Archetype,
            Self::OpenehrRoot,
            Self::Composition,
            Self::Reference,
            Self::Context,
        ];
        admitted
            .into_iter()
            .find(|variable| name.eq_ignore_ascii_case(variable.as_str()))
            .ok_or_else(|| KeywordError {
                value: value.to_owned(),
                admitted: Self::ADMITTED,
            })
    }
}

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

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::ConditionOperator;
    use super::DataType;
    use super::Direction;
    use super::ExtensionMethod;
    use super::Variable;

    #[test]
    fn both_documented_direction_spellings_parse() {
        assert_eq!(
            Direction::from_str("openehr->fhir"),
            Ok(Direction::OpenehrToFhir)
        );
        assert_eq!(
            Direction::from_str("fhir->openehr"),
            Ok(Direction::FhirToOpenehr)
        );
    }

    #[test]
    fn a_direction_compares_case_insensitively() {
        assert_eq!(
            Direction::from_str("openEHR->fhir"),
            Ok(Direction::OpenehrToFhir)
        );
        assert_eq!(
            Direction::from_str("fhir->openEHR"),
            Ok(Direction::FhirToOpenehr)
        );
    }

    #[test]
    fn a_direction_outside_the_set_is_refused_naming_the_set() {
        let error = Direction::from_str("openehr<->fhir").expect_err("a value outside the set");
        assert_eq!(error.value, "openehr<->fhir");
        assert_eq!(
            error.to_string(),
            "`openehr<->fhir` is not one of `openehr->fhir`, `fhir->openehr`"
        );
    }

    #[test]
    fn an_extension_method_is_case_exact() {
        assert_eq!(ExtensionMethod::from_str("add"), Ok(ExtensionMethod::Add));
        assert!(ExtensionMethod::from_str("Add").is_err());
    }

    #[test]
    fn a_data_type_is_case_exact() {
        assert_eq!(DataType::from_str("NONE"), Ok(DataType::None));
        assert!(DataType::from_str("none").is_err());
    }

    #[test]
    fn an_operator_is_case_exact_and_knows_whether_it_takes_criteria() {
        assert_eq!(
            ConditionOperator::from_str("not empty"),
            Ok(ConditionOperator::NotEmpty)
        );
        assert!(!ConditionOperator::NotEmpty.takes_criteria());
        assert!(!ConditionOperator::Empty.takes_criteria());
        assert!(ConditionOperator::OneOf.takes_criteria());
        assert!(ConditionOperator::NotOf.takes_criteria());
        assert!(ConditionOperator::Type.takes_criteria());
        assert!(ConditionOperator::from_str("One Of").is_err());
    }

    #[test]
    fn a_variable_compares_case_insensitively_with_or_without_the_sigil() {
        assert_eq!(
            Variable::from_str("$openehrRoot"),
            Ok(Variable::OpenehrRoot)
        );
        assert_eq!(
            Variable::from_str("$openEHRRoot"),
            Ok(Variable::OpenehrRoot)
        );
        assert_eq!(Variable::from_str("archetype"), Ok(Variable::Archetype));
        assert_eq!(Variable::OpenehrRoot.to_string(), "$openehrRoot");
    }

    #[test]
    fn a_variable_outside_the_set_is_refused() {
        let error = Variable::from_str("$openehrParent").expect_err("a name outside the set");
        assert_eq!(error.admitted, Variable::ADMITTED);
    }
}
