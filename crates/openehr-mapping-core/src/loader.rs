// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The YAML loader: one file in, a header and a positioned value tree out.
//!
//! The parser is `serde-saphyr`
//! (<https://docs.rs/serde-saphyr/1.2.0/serde_saphyr/>), which resolves
//! anchors, aliases and merge keys and reports 1-based character positions for
//! both values and errors. The OMOCL corpus needs the first of those: 57 of
//! its 202 files define an anchor.
//!
//! Four parser settings are deliberate. Duplicate keys are an error rather
//! than a last-writer-wins overwrite, merge keys are expanded, an unrecognised
//! YAML tag is refused, and the snippet wrapper is off because a
//! [`crate::diagnostic::Diagnostic`] carries the position itself.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::DiagnosticCode;
use crate::header::Header;
use crate::header::HeaderError;
use crate::position::Position;
use crate::value::PositionedValue;

/// One loaded mapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingDocument {
    file: PathBuf,
    header: Header,
    document: PositionedValue,
}

impl MappingDocument {
    /// Returns the file the document was loaded from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Returns the shared header.
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// Returns the whole document as a positioned value tree, header included.
    #[must_use]
    pub const fn document(&self) -> &PositionedValue {
        &self.document
    }
}

/// Why a mapping file could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("cannot read {}", file.display())]
    Read {
        /// The file that could not be read.
        file: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
    /// The YAML could not be parsed.
    #[error("cannot parse {}", file.display())]
    Yaml {
        /// The file that could not be parsed.
        file: PathBuf,
        /// Where the parser stopped, when it reported a position.
        position: Option<Position>,
        /// The underlying failure.
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    /// The parsed document carries no usable shared header.
    #[error("the header of {} was refused", file.display())]
    Header {
        /// The file whose header was refused.
        file: PathBuf,
        /// The underlying failure.
        #[source]
        source: HeaderError,
    },
}

impl LoadError {
    /// Returns the file the failure is about.
    #[must_use]
    pub fn file(&self) -> &Path {
        match *self {
            Self::Read { ref file, .. }
            | Self::Yaml { ref file, .. }
            | Self::Header { ref file, .. } => file,
        }
    }

    /// Returns the code this refusal reports.
    #[must_use]
    pub fn code(&self) -> DiagnosticCode {
        match *self {
            Self::Read { .. } => DiagnosticCode::Unreadable,
            Self::Yaml { ref source, .. } => yaml_code(source),
            Self::Header { ref source, .. } => source.code(),
        }
    }
}

impl From<&LoadError> for Diagnostic {
    fn from(error: &LoadError) -> Self {
        match *error {
            LoadError::Read {
                ref file,
                ref source,
            } => Self::error(file.clone(), DiagnosticCode::Unreadable, source.to_string()),
            LoadError::Yaml {
                ref file,
                position,
                ref source,
            } => Self::error(file.clone(), yaml_code(source), source.to_string())
                .with_optional_position(position),
            LoadError::Header {
                ref file,
                ref source,
            } => source.to_diagnostic(file.clone()),
        }
    }
}

/// Maps a parser failure onto the diagnostic code that names it.
fn yaml_code(error: &serde_saphyr::Error) -> DiagnosticCode {
    match *error {
        serde_saphyr::Error::DuplicateMappingKey { .. } => DiagnosticCode::YamlDuplicateKey,
        serde_saphyr::Error::UnknownAnchor { .. } => DiagnosticCode::YamlUnresolvedAlias,
        _ => DiagnosticCode::YamlSyntax,
    }
}

/// Returns the parser settings every mapping file is read with.
fn parser_options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        with_snippet: false,
        reject_unsupported_tags: true,
    }
}

/// Parses one YAML document into a positioned value tree, without reading a
/// header from it.
///
/// # Errors
///
/// Returns [`LoadError::Yaml`] for any parse failure, including a duplicate
/// key, an alias with no anchor, and a tab used for indentation.
pub fn parse_str(file: impl Into<PathBuf>, source: &str) -> Result<PositionedValue, LoadError> {
    let file = file.into();
    match serde_saphyr::from_str_with_options::<PositionedValue>(source, parser_options()) {
        Ok(document) => Ok(document),
        Err(error) => {
            let position = error.location().map(Position::from);
            Err(LoadError::Yaml {
                file,
                position,
                source: Box::new(error),
            })
        }
    }
}

/// Loads one mapping file from a string.
///
/// `file` is the path the diagnostics name; the bytes come from `source`.
///
/// # Errors
///
/// Returns [`LoadError::Yaml`] for a parse failure and [`LoadError::Header`]
/// when the parsed document carries no usable shared header.
pub fn load_str(file: impl Into<PathBuf>, source: &str) -> Result<MappingDocument, LoadError> {
    let file = file.into();
    let document = parse_str(file.clone(), source)?;
    let header = Header::from_document(&document).map_err(|source| LoadError::Header {
        file: file.clone(),
        source,
    })?;
    Ok(MappingDocument {
        file,
        header,
        document,
    })
}

/// Loads one mapping file from disk.
///
/// # Errors
///
/// Returns [`LoadError::Read`] when the file cannot be read, and otherwise
/// whatever [`load_str`] returns.
pub fn load_file(file: impl Into<PathBuf>) -> Result<MappingDocument, LoadError> {
    let file = file.into();
    let source = std::fs::read_to_string(&file).map_err(|source| LoadError::Read {
        file: file.clone(),
        source,
    })?;
    load_str(file, &source)
}

#[cfg(test)]
mod tests {
    use super::load_str;
    use super::parse_str;
    use crate::diagnostic::Diagnostic;
    use crate::diagnostic::DiagnosticCode;
    use crate::header::MappingLanguage;
    use crate::header::MappingType;
    use crate::position::Position;

    const MINIMAL: &str = "grammar: OMOCL/v1.0.0\n\
type: model\n\
metadata:\n\
\x20 name: person_data.v0\n\
\x20 version: 1.0.0\n\
spec:\n\
\x20 system: OMOP\n\
\x20 version: 5.4\n\
\x20 openEhrConfig:\n\
\x20   archetype: openEHR-EHR-ADMIN_ENTRY.person_data.v0\n";

    #[test]
    fn a_minimal_omocl_header_loads() {
        let document = load_str("person_data.yml", MINIMAL).expect("a well-formed header");
        let header = document.header();
        assert_eq!(header.grammar().value().language(), MappingLanguage::Omocl);
        assert_eq!(header.name().value().as_str(), "person_data.v0");
        assert_eq!(header.version().value().as_str(), "1.0.0");
        assert_eq!(
            header.mapping_type().map(|t| *t.value()),
            Some(MappingType::Model)
        );
        assert_eq!(
            header.archetype().map(|a| a.value().as_str()),
            Some("openEHR-EHR-ADMIN_ENTRY.person_data.v0")
        );
        assert_eq!(header.name().position(), Position::new(4, 9));
    }

    #[test]
    fn the_whole_document_is_positioned() {
        let document = parse_str("person_data.yml", MINIMAL).expect("well-formed YAML");
        let grammar = document.get("grammar").expect("a grammar node");
        assert_eq!(grammar.position(), Position::new(1, 10));
        let entry = document.entry("grammar").expect("a grammar entry");
        assert_eq!(entry.key_position(), Position::new(1, 1));
    }

    #[test]
    fn a_duplicate_key_is_refused_with_its_position() {
        let error = parse_str("dup.yml", "a: 1\nb: 2\na: 3\n").expect_err("a duplicate key");
        assert_eq!(error.code(), DiagnosticCode::YamlDuplicateKey);
        let diagnostic = Diagnostic::from(&error);
        assert_eq!(diagnostic.position(), Some(Position::new(3, 1)));
    }

    #[test]
    fn an_alias_without_an_anchor_is_refused() {
        let error = parse_str("alias.yml", "a: *missing\n").expect_err("an unresolvable alias");
        assert_eq!(error.code(), DiagnosticCode::YamlUnresolvedAlias);
    }
}
