// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The header both mapping languages share.
//!
//! The FHIRconnect header page states that the header "is standardized for
//! both FHIRconnect and OMOCL"
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Header). It
//! carries `grammar`, `type`, `metadata.name`, `metadata.version` and `spec`,
//! with `spec.openEhrConfig.archetype` naming the mapped archetype. Everything
//! below `spec` that a language adds to it (`fhirConfig`, `extends`, `system`,
//! `version`) stays an opaque positioned node for that language's crate to
//! read.

use core::fmt;
use core::str::FromStr;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::diagnostic::ModelPath;
use crate::position::Located;
use crate::position::Position;
use crate::value::MappingEntry;
use crate::value::MappingValue;
use crate::value::PositionedValue;
use crate::value::ValueKind;

/// One of the two mapping languages a header can declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingLanguage {
    /// FHIRconnect, the openEHR to HL7 FHIR mapping language.
    FhirConnect,
    /// OMOCL, the openEHR to OMOP Common Data Model mapping language.
    Omocl,
}

impl MappingLanguage {
    /// Returns the spelling the language's own schema or corpus writes.
    ///
    /// FHIRconnect's published schemas pin the spelling with the pattern
    /// `^FHIRConnect/v[0-9]+\.[0-9]+\.[0-9]+$`
    /// (`docs/specs/fhirconnect/build/site/FHIRconnect/v1.0.0/_attachments/model-mapping.schema.json`,
    /// `properties.grammar.pattern`); every OMOCL corpus file writes `OMOCL`.
    #[must_use]
    pub const fn canonical_spelling(self) -> &'static str {
        match self {
            Self::FhirConnect => "FHIRConnect",
            Self::Omocl => "OMOCL",
        }
    }
}

impl fmt::Display for MappingLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.canonical_spelling())
    }
}

/// The three-part version a `grammar` value carries after the `v`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrammarSemVer {
    major: u32,
    minor: u32,
    patch: u32,
}

impl GrammarSemVer {
    /// Creates a grammar version.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Returns the major version.
    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the minor version.
    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Returns the patch version.
    #[must_use]
    pub const fn patch(self) -> u32 {
        self.patch
    }
}

impl fmt::Display for GrammarSemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Why a `grammar` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrammarVersionError {
    /// The value carries no `/` separating the language from the version.
    #[error("`{value}` is not `<language>/v<major>.<minor>.<patch>`")]
    MissingSeparator {
        /// The refused value.
        value: String,
    },
    /// The language is neither FHIRconnect nor OMOCL.
    #[error("`{language}` is neither FHIRConnect nor OMOCL")]
    UnknownLanguage {
        /// The refused language spelling.
        language: String,
    },
    /// The version is outside the `vMAJOR.MINOR.PATCH` shape.
    #[error("`{version}` is not `v<major>.<minor>.<patch>`")]
    MalformedVersion {
        /// The refused version.
        version: String,
    },
}

/// The `grammar` value: a language and the version of its grammar.
///
/// The language spelling is compared case-insensitively and kept verbatim. The
/// specification's own text is case-inconsistent about its keyword values
/// (`docs/architecture.md` records the adjudication, and the FHIRconnect
/// schema pattern writes `FHIRConnect` where the specification title writes
/// FHIRconnect), so a case-exact comparison would refuse a file the
/// specification's own prose spells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarVersion {
    language: MappingLanguage,
    spelling: String,
    version: GrammarSemVer,
}

impl GrammarVersion {
    /// Returns the language.
    #[must_use]
    pub const fn language(&self) -> MappingLanguage {
        self.language
    }

    /// Returns the language spelling as the file writes it.
    #[must_use]
    pub fn spelling(&self) -> &str {
        &self.spelling
    }

    /// Returns the grammar version.
    #[must_use]
    pub const fn version(&self) -> GrammarSemVer {
        self.version
    }
}

impl fmt::Display for GrammarVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.spelling, self.version)
    }
}

impl FromStr for GrammarVersion {
    type Err = GrammarVersionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((spelling, version)) = value.split_once('/') else {
            return Err(GrammarVersionError::MissingSeparator {
                value: value.to_owned(),
            });
        };
        let language =
            if spelling.eq_ignore_ascii_case(MappingLanguage::FhirConnect.canonical_spelling()) {
                MappingLanguage::FhirConnect
            } else if spelling.eq_ignore_ascii_case(MappingLanguage::Omocl.canonical_spelling()) {
                MappingLanguage::Omocl
            } else {
                return Err(GrammarVersionError::UnknownLanguage {
                    language: spelling.to_owned(),
                });
            };
        let version =
            parse_grammar_semver(version).ok_or_else(|| GrammarVersionError::MalformedVersion {
                version: version.to_owned(),
            })?;
        Ok(Self {
            language,
            spelling: spelling.to_owned(),
            version,
        })
    }
}

/// Parses `v<major>.<minor>.<patch>`, the shape both grammar patterns write.
fn parse_grammar_semver(version: &str) -> Option<GrammarSemVer> {
    let digits = version.strip_prefix('v')?;
    let mut parts = digits.split('.');
    let major = parse_decimal(parts.next()?)?;
    let minor = parse_decimal(parts.next()?)?;
    let patch = parse_decimal(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(GrammarSemVer::new(major, minor, patch))
}

/// Parses one `[0-9]+` field of a grammar version.
fn parse_decimal(field: &str) -> Option<u32> {
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

/// Why a `metadata.name` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingNameError {
    /// The name was empty.
    #[error("a mapping name is not empty")]
    Empty,
    /// The name has leading or trailing whitespace.
    #[error("the mapping name `{name}` has leading or trailing whitespace")]
    Untrimmed {
        /// The refused name.
        name: String,
    },
}

/// The `metadata.name` value: the id a mapping is referenced by.
///
/// The FHIRconnect header page calls the name "a unique id used to identify
/// the mapping and reference it"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Metadata),
/// and every cross-file reference resolves against it by exact string. The
/// published schemas constrain the value only to a string, so refusing an
/// empty or whitespace-padded name is FerroBRIDGE's own rule: neither can be
/// referenced unambiguously.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MappingName(String);

impl MappingName {
    /// Creates a mapping name.
    ///
    /// # Errors
    ///
    /// Returns [`MappingNameError`] when the name is empty or carries leading
    /// or trailing whitespace.
    pub fn new(name: impl Into<String>) -> Result<Self, MappingNameError> {
        let name = name.into();
        if name.is_empty() {
            return Err(MappingNameError::Empty);
        }
        if name.trim() != name {
            return Err(MappingNameError::Untrimmed { name });
        }
        Ok(Self(name))
    }

    /// Returns the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MappingName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The `metadata.version` value, kept exactly as the file writes it.
///
/// Both published schemas type the field as a plain string with no pattern
/// (`model-mapping.schema.json`, `properties.metadata.properties.version`), so
/// no version grammar is imposed on it here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MappingVersion(String);

impl MappingVersion {
    /// Creates a mapping version from its lexical form.
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    /// Returns the lexical form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MappingVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a `type` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not `model`, `extension` or `context`")]
pub struct MappingTypeError {
    /// The refused value.
    pub value: String,
}

/// The `type` value: which of the three mapping-file kinds this file is.
///
/// FHIRconnect defines exactly three
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/main.adoc`):
/// the model mapping between an archetype and an unprofiled resource, the
/// extension mapping that adds to a model mapping for a profile and template,
/// and the context mapping that imports the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MappingType {
    /// An archetype to unprofiled-resource mapping.
    Model,
    /// An addition to a model mapping, for one profile and template.
    Extension,
    /// The import file that names a profile, a template and a starting
    /// mapping.
    Context,
}

impl MappingType {
    /// Returns the spelling the schema enum fixes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Extension => "extension",
            Self::Context => "context",
        }
    }
}

impl fmt::Display for MappingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MappingType {
    type Err = MappingTypeError;

    /// Parses the value case-exactly.
    ///
    /// Both published schemas fix the value with a JSON Schema `enum`
    /// (`model-mapping.schema.json`, `properties.type.enum`), and a JSON
    /// Schema enum matches case-sensitively, so `Model` is not this value.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "model" => Ok(Self::Model),
            "extension" => Ok(Self::Extension),
            "context" => Ok(Self::Context),
            _ => Err(MappingTypeError {
                value: value.to_owned(),
            }),
        }
    }
}

/// Why a `spec.openEhrConfig.archetype` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArchetypeIdError {
    /// The value is not `<qualified_rm_entity>.<domain_concept>.<version_id>`.
    #[error("`{value}` is not `rm_originator-rm_name-rm_entity.concept.vN`")]
    Shape {
        /// The refused value.
        value: String,
    },
    /// The first part is not `rm_originator-rm_name-rm_entity`.
    #[error("`{value}` is not `rm_originator-rm_name-rm_entity`")]
    QualifiedRmEntity {
        /// The refused first part.
        value: String,
    },
    /// The second part is not a concept name with optional specialisations.
    #[error("`{value}` is not `concept_name{{-specialisation}}`")]
    DomainConcept {
        /// The refused second part.
        value: String,
    },
    /// The third part is not `v` followed by a version number.
    #[error("`{value}` is not `v0` or `v` followed by a non-zero-leading number")]
    VersionId {
        /// The refused third part.
        value: String,
    },
}

/// An openEHR archetype id.
///
/// The lexical form is `rm_originator '-' rm_name '-' rm_entity '.'
/// concept_name {'-' specialisation}* '.v' number`, and the syntax appendix
/// gives `version-id = 'v', ('0' | non-zero-digit, [number])` and
/// `alphanum-str = letter, {letter | digit | '_'}` (openEHR BASE Release
/// 1.2.0, §5.4.10 `ARCHETYPE_ID` Class and the Base Types syntax appendix,
/// <https://specifications.openehr.org/releases/BASE/Release-1.2.0/base_types.html>).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchetypeId {
    value: String,
    version: u32,
}

impl ArchetypeId {
    /// Returns the archetype id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns the number of the `version_id` axis.
    #[must_use]
    pub const fn version_number(&self) -> u32 {
        self.version
    }
}

impl fmt::Display for ArchetypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl FromStr for ArchetypeId {
    type Err = ArchetypeIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split('.');
        let (Some(qualified_rm_entity), Some(domain_concept), Some(version_id), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ArchetypeIdError::Shape {
                value: value.to_owned(),
            });
        };

        let axes: Vec<&str> = qualified_rm_entity.split('-').collect();
        if axes.len() != 3 || !axes.iter().all(|axis| is_alphanum_str(axis)) {
            return Err(ArchetypeIdError::QualifiedRmEntity {
                value: qualified_rm_entity.to_owned(),
            });
        }

        if !domain_concept.split('-').all(is_alphanum_str) {
            return Err(ArchetypeIdError::DomainConcept {
                value: domain_concept.to_owned(),
            });
        }

        let version = parse_version_id(version_id).ok_or_else(|| ArchetypeIdError::VersionId {
            value: version_id.to_owned(),
        })?;

        Ok(Self {
            value: value.to_owned(),
            version,
        })
    }
}

/// Whether a string is `letter, {letter | digit | '_'}`.
fn is_alphanum_str(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parses `version-id = 'v', ('0' | non-zero-digit, [number])`.
fn parse_version_id(version_id: &str) -> Option<u32> {
    let digits = version_id.strip_prefix('v')?;
    if digits == "0" {
        return Some(0);
    }
    let mut characters = digits.chars();
    let first = characters.next()?;
    if !first.is_ascii_digit() || first == '0' {
        return None;
    }
    if !characters.all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Why a header could not be read from a loaded document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    /// The document holds no node at all.
    #[error("the document is empty")]
    EmptyDocument,
    /// A node is not of the kind the header needs there.
    #[error("expected {expected} at `{path}`, found {found}")]
    UnexpectedNodeKind {
        /// Where in the document model the node sits.
        path: ModelPath,
        /// Where in the file the node sits.
        position: Position,
        /// The kind the header needs.
        expected: ValueKind,
        /// The kind the file wrote.
        found: ValueKind,
    },
    /// A key the header needs is absent.
    #[error("the header has no `{path}` key")]
    MissingKey {
        /// The absent key, as a path into the document model.
        path: ModelPath,
        /// Where the enclosing node sits.
        position: Position,
    },
    /// The `grammar` value was refused.
    #[error("the `{path}` value was refused")]
    Grammar {
        /// Where in the document model the value sits.
        path: ModelPath,
        /// Where in the file the value sits.
        position: Position,
        /// Why it was refused.
        #[source]
        source: GrammarVersionError,
    },
    /// The `type` value was refused.
    #[error("the `{path}` value was refused")]
    MappingType {
        /// Where in the document model the value sits.
        path: ModelPath,
        /// Where in the file the value sits.
        position: Position,
        /// Why it was refused.
        #[source]
        source: MappingTypeError,
    },
    /// A FHIRconnect file has no `type` key.
    ///
    /// Both published FHIRconnect schemas list `type` in their top-level
    /// `required` array. OMOCL publishes no schema, so this rule is
    /// FHIRconnect's alone.
    #[error("a FHIRconnect mapping file declares `type`")]
    MissingMappingType {
        /// Where the document root sits.
        position: Position,
    },
    /// The `metadata.name` value was refused.
    #[error("the `{path}` value was refused")]
    MappingName {
        /// Where in the document model the value sits.
        path: ModelPath,
        /// Where in the file the value sits.
        position: Position,
        /// Why it was refused.
        #[source]
        source: MappingNameError,
    },
    /// The `spec.openEhrConfig.archetype` value was refused.
    #[error("the `{path}` value was refused")]
    ArchetypeId {
        /// Where in the document model the value sits.
        path: ModelPath,
        /// Where in the file the value sits.
        position: Position,
        /// Why it was refused.
        #[source]
        source: ArchetypeIdError,
    },
}

impl HeaderError {
    /// Returns the code this refusal reports.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        match *self {
            Self::EmptyDocument => DiagnosticCode::EmptyDocument,
            Self::UnexpectedNodeKind { .. } => DiagnosticCode::UnexpectedNodeKind,
            Self::MissingKey { .. } | Self::MissingMappingType { .. } => DiagnosticCode::MissingKey,
            Self::Grammar { ref source, .. } => match *source {
                GrammarVersionError::UnknownLanguage { .. } => {
                    DiagnosticCode::UnknownGrammarLanguage
                }
                GrammarVersionError::MissingSeparator { .. }
                | GrammarVersionError::MalformedVersion { .. } => {
                    DiagnosticCode::MalformedGrammarVersion
                }
            },
            Self::MappingType { .. } => DiagnosticCode::UnknownMappingType,
            Self::MappingName { .. } => DiagnosticCode::InvalidMappingName,
            Self::ArchetypeId { .. } => DiagnosticCode::InvalidArchetypeId,
        }
    }

    /// Returns where in the file the refusal happened, when a position is
    /// known.
    #[must_use]
    pub const fn position(&self) -> Option<Position> {
        match *self {
            Self::EmptyDocument => None,
            Self::UnexpectedNodeKind { position, .. }
            | Self::MissingKey { position, .. }
            | Self::Grammar { position, .. }
            | Self::MappingType { position, .. }
            | Self::MissingMappingType { position }
            | Self::MappingName { position, .. }
            | Self::ArchetypeId { position, .. } => Some(position),
        }
    }

    /// Returns where in the document model the refusal happened.
    #[must_use]
    pub fn model_path(&self) -> ModelPath {
        match *self {
            Self::EmptyDocument | Self::MissingMappingType { .. } => ModelPath::root(),
            Self::UnexpectedNodeKind { ref path, .. }
            | Self::MissingKey { ref path, .. }
            | Self::Grammar { ref path, .. }
            | Self::MappingType { ref path, .. }
            | Self::MappingName { ref path, .. }
            | Self::ArchetypeId { ref path, .. } => path.clone(),
        }
    }

    /// Renders this refusal as a diagnostic about `file`.
    #[must_use]
    pub fn to_diagnostic(&self, file: impl Into<std::path::PathBuf>) -> Diagnostic {
        Diagnostic::error(file, self.code(), self.to_string())
            .with_optional_position(self.position())
            .with_model_path(self.model_path())
    }
}

/// The header of one mapping file.
///
/// Every modelled field keeps the position it was read from, so a refusal that
/// is about the header points at the header. `spec` is kept whole as an opaque
/// node beside the fields read out of it, because what a language adds under
/// it (`fhirConfig`, `extends`, `system`, `version`) is that language's to
/// interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    grammar: Located<GrammarVersion>,
    mapping_type: Option<Located<MappingType>>,
    name: Located<MappingName>,
    version: Located<MappingVersion>,
    archetype: Option<Located<ArchetypeId>>,
    revision: Option<PositionedValue>,
    spec: PositionedValue,
}

impl Header {
    /// Reads the shared header out of a loaded document.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderError`] when the document is empty, is not a mapping,
    /// omits a key the header needs, or writes a value the header model
    /// refuses.
    pub fn from_document(document: &PositionedValue) -> Result<Self, HeaderError> {
        let root = ModelPath::root();
        if matches!(*document.value(), MappingValue::Null) {
            return Err(HeaderError::EmptyDocument);
        }
        require_mapping(document, &root)?;

        let grammar_path = root.field("grammar");
        let grammar_entry = require_entry(document, &root, "grammar")?;
        let grammar_text = require_text(grammar_entry.value(), &grammar_path)?;
        let grammar =
            GrammarVersion::from_str(grammar_text).map_err(|source| HeaderError::Grammar {
                path: grammar_path.clone(),
                position: grammar_entry.value().position(),
                source,
            })?;

        let mapping_type = match document.entry("type") {
            Some(entry) => {
                let path = root.field("type");
                let text = require_text(entry.value(), &path)?;
                let parsed =
                    MappingType::from_str(text).map_err(|source| HeaderError::MappingType {
                        path,
                        position: entry.value().position(),
                        source,
                    })?;
                Some(Located::new(entry.value().position(), parsed))
            }
            None if grammar.language() == MappingLanguage::FhirConnect => {
                return Err(HeaderError::MissingMappingType {
                    position: document.position(),
                });
            }
            None => None,
        };

        let metadata_path = root.field("metadata");
        let metadata = require_entry(document, &root, "metadata")?.value();
        require_mapping(metadata, &metadata_path)?;

        let name_path = metadata_path.field("name");
        let name_entry = require_entry(metadata, &metadata_path, "name")?;
        let name_text = require_text(name_entry.value(), &name_path)?;
        let name = MappingName::new(name_text).map_err(|source| HeaderError::MappingName {
            path: name_path,
            position: name_entry.value().position(),
            source,
        })?;

        let version_path = metadata_path.field("version");
        let version_entry = require_entry(metadata, &metadata_path, "version")?;
        let version_text = require_text(version_entry.value(), &version_path)?;

        let spec_path = root.field("spec");
        let spec = require_entry(document, &root, "spec")?.value();
        require_mapping(spec, &spec_path)?;

        let (archetype, revision) = match spec.entry("openEhrConfig") {
            Some(entry) => {
                let config_path = spec_path.field("openEhrConfig");
                let config = entry.value();
                require_mapping(config, &config_path)?;
                let archetype = match config.entry("archetype") {
                    Some(archetype_entry) => {
                        let path = config_path.field("archetype");
                        let text = require_text(archetype_entry.value(), &path)?;
                        let parsed = ArchetypeId::from_str(text).map_err(|source| {
                            HeaderError::ArchetypeId {
                                path,
                                position: archetype_entry.value().position(),
                                source,
                            }
                        })?;
                        Some(Located::new(archetype_entry.value().position(), parsed))
                    }
                    None => None,
                };
                (archetype, config.get("revision").cloned())
            }
            None => (None, None),
        };

        Ok(Self {
            grammar: Located::new(grammar_entry.value().position(), grammar),
            mapping_type,
            name: Located::new(name_entry.value().position(), name),
            version: Located::new(
                version_entry.value().position(),
                MappingVersion::new(version_text),
            ),
            archetype,
            revision,
            spec: spec.clone(),
        })
    }

    /// Returns the declared grammar.
    #[must_use]
    pub const fn grammar(&self) -> &Located<GrammarVersion> {
        &self.grammar
    }

    /// Returns the declared file type, which OMOCL's unschema'd header may
    /// omit.
    #[must_use]
    pub const fn mapping_type(&self) -> Option<&Located<MappingType>> {
        self.mapping_type.as_ref()
    }

    /// Returns the mapping name.
    #[must_use]
    pub const fn name(&self) -> &Located<MappingName> {
        &self.name
    }

    /// Returns the mapping version.
    #[must_use]
    pub const fn version(&self) -> &Located<MappingVersion> {
        &self.version
    }

    /// Returns the mapped archetype, which a context mapping has none of.
    #[must_use]
    pub const fn archetype(&self) -> Option<&Located<ArchetypeId>> {
        self.archetype.as_ref()
    }

    /// Returns `spec.openEhrConfig.revision` as the opaque node the file
    /// writes.
    ///
    /// The FHIRconnect header page says the field "states what revision of the
    /// archetype this mapping applies for"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Spec)
    /// and fixes no grammar for it, so it is carried, not parsed.
    #[must_use]
    pub const fn revision(&self) -> Option<&PositionedValue> {
        self.revision.as_ref()
    }

    /// Returns the whole `spec` node, including everything a language adds to
    /// it.
    #[must_use]
    pub const fn spec(&self) -> &PositionedValue {
        &self.spec
    }
}

/// Refuses a node that is not a mapping.
fn require_mapping(node: &PositionedValue, path: &ModelPath) -> Result<(), HeaderError> {
    if node.as_mapping().is_some() {
        return Ok(());
    }
    Err(HeaderError::UnexpectedNodeKind {
        path: path.clone(),
        position: node.position(),
        expected: ValueKind::Mapping,
        found: node.kind(),
    })
}

/// Reads a required key out of a mapping node.
fn require_entry<'a>(
    node: &'a PositionedValue,
    path: &ModelPath,
    key: &str,
) -> Result<&'a MappingEntry, HeaderError> {
    node.entry(key).ok_or_else(|| HeaderError::MissingKey {
        path: path.field(key),
        position: node.position(),
    })
}

/// Reads a string node.
fn require_text<'a>(node: &'a PositionedValue, path: &ModelPath) -> Result<&'a str, HeaderError> {
    node.as_text()
        .ok_or_else(|| HeaderError::UnexpectedNodeKind {
            path: path.clone(),
            position: node.position(),
            expected: ValueKind::Text,
            found: node.kind(),
        })
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::ArchetypeId;
    use super::ArchetypeIdError;
    use super::GrammarSemVer;
    use super::GrammarVersion;
    use super::GrammarVersionError;
    use super::MappingLanguage;
    use super::MappingName;
    use super::MappingNameError;
    use super::MappingType;

    #[test]
    fn the_two_grammar_languages_parse() {
        let fhir = GrammarVersion::from_str("FHIRConnect/v1.0.0").expect("a FHIRconnect grammar");
        assert_eq!(fhir.language(), MappingLanguage::FhirConnect);
        assert_eq!(fhir.version(), GrammarSemVer::new(1, 0, 0));
        let omocl = GrammarVersion::from_str("OMOCL/v1.0.0").expect("an OMOCL grammar");
        assert_eq!(omocl.language(), MappingLanguage::Omocl);
    }

    #[test]
    fn the_language_spelling_is_case_insensitive_and_kept_verbatim() {
        let schema_case = GrammarVersion::from_str("FHIRConnect/v1.0.0").expect("schema spelling");
        let prose_case = GrammarVersion::from_str("FHIRconnect/v1.0.0").expect("prose spelling");
        assert_eq!(schema_case.version(), prose_case.version());
        assert_eq!(schema_case.language(), prose_case.language());
        assert_eq!(prose_case.spelling(), "FHIRconnect");
        assert_eq!(prose_case.to_string(), "FHIRconnect/v1.0.0");
    }

    #[test]
    fn a_two_part_version_and_an_unknown_language_are_refused() {
        assert_eq!(
            GrammarVersion::from_str("FHIRConnect/1.0"),
            Err(GrammarVersionError::MalformedVersion {
                version: "1.0".to_owned()
            })
        );
        assert_eq!(
            GrammarVersion::from_str("Foo/v1.0.0"),
            Err(GrammarVersionError::UnknownLanguage {
                language: "Foo".to_owned()
            })
        );
        assert_eq!(
            GrammarVersion::from_str("OMOCL"),
            Err(GrammarVersionError::MissingSeparator {
                value: "OMOCL".to_owned()
            })
        );
    }

    #[test]
    fn the_file_types_parse_case_exactly() {
        assert_eq!(MappingType::from_str("model"), Ok(MappingType::Model));
        assert_eq!(MappingType::from_str("context"), Ok(MappingType::Context));
        assert!(MappingType::from_str("Model").is_err());
    }

    #[test]
    fn an_archetype_id_follows_the_base_lexical_form() {
        let id = ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis.v1")
            .expect("a well-formed archetype id");
        assert_eq!(id.version_number(), 1);
        assert_eq!(id.as_str(), "openEHR-EHR-EVALUATION.problem_diagnosis.v1");
        assert!(ArchetypeId::from_str("openEHR-EHR-CLUSTER.imaging_exam-foetus.v1").is_ok());
        assert!(ArchetypeId::from_str("openEHR-EHR-ACTION.informed_consent.v0").is_ok());
    }

    #[test]
    fn an_archetype_id_without_a_version_is_refused() {
        assert_eq!(
            ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis"),
            Err(ArchetypeIdError::Shape {
                value: "openEHR-EHR-EVALUATION.problem_diagnosis".to_owned()
            })
        );
        assert_eq!(
            ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis.1"),
            Err(ArchetypeIdError::VersionId {
                value: "1".to_owned()
            })
        );
        assert_eq!(
            ArchetypeId::from_str("openEHR-EHR-EVALUATION.problem_diagnosis.v01"),
            Err(ArchetypeIdError::VersionId {
                value: "v01".to_owned()
            })
        );
        assert_eq!(
            ArchetypeId::from_str("openEHR-EHR.problem_diagnosis.v1"),
            Err(ArchetypeIdError::QualifiedRmEntity {
                value: "openEHR-EHR".to_owned()
            })
        );
    }

    #[test]
    fn a_mapping_name_is_a_referenceable_id() {
        assert_eq!(
            MappingName::new("  padded  "),
            Err(MappingNameError::Untrimmed {
                name: "  padded  ".to_owned()
            })
        );
        assert_eq!(MappingName::new(""), Err(MappingNameError::Empty));
        assert_eq!(
            MappingName::new("ACTION.procedure.v1")
                .expect("a well-formed name")
                .as_str(),
            "ACTION.procedure.v1"
        );
    }
}
