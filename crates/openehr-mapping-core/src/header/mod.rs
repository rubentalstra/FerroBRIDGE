// SPDX-FileCopyrightText: Vernum Projecten B.V.
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

pub mod archetype;
pub mod grammar;
pub mod metadata;
#[cfg(test)]
mod tests;

use core::str::FromStr;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::diagnostic::ModelPath;
use crate::header::archetype::ArchetypeId;
use crate::header::archetype::ArchetypeIdError;
use crate::header::grammar::GrammarVersion;
use crate::header::grammar::GrammarVersionError;
use crate::header::grammar::MappingLanguage;
use crate::header::metadata::MappingName;
use crate::header::metadata::MappingNameError;
use crate::header::metadata::MappingType;
use crate::header::metadata::MappingTypeError;
use crate::header::metadata::MappingVersion;
use crate::position::Located;
use crate::position::Position;
use crate::value::MappingEntry;
use crate::value::MappingValue;
use crate::value::PositionedValue;
use crate::value::ValueKind;

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
