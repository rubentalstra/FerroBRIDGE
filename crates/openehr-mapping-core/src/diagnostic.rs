// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The one diagnostic every refusal renders into.
//!
//! No specification governs diagnostics: this is FerroBRIDGE's own design. A
//! diagnostic carries where the refusal happened (the file, the YAML line and
//! column, the mapping name, the path into the document model), how bad it is,
//! a stable code, and one sentence of message.

use core::fmt;
use std::path::Path;
use std::path::PathBuf;

use crate::header::MappingName;
use crate::position::Position;

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The input is refused.
    Error,
    /// The input is accepted, and something about it is worth reporting.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match *self {
            Self::Error => "error",
            Self::Warning => "warning",
        };
        f.write_str(name)
    }
}

/// Why a language diagnostic code was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LanguageCodeError {
    /// The code was empty.
    #[error("a language diagnostic code is not empty")]
    Empty,
    /// The code carries a character outside the code alphabet.
    #[error(
        "a language diagnostic code is lowercase ASCII letters, digits and hyphens, \
         and `{code}` carries `{character}`"
    )]
    Character {
        /// The refused code.
        code: String,
        /// The first character outside the alphabet.
        character: char,
    },
}

/// A diagnostic code raised by a language crate built on this foundation.
///
/// The code alphabet is lowercase ASCII letters, digits and hyphens, so a code
/// reads the same in a log line, a JSON field and a terminal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LanguageCode(String);

impl LanguageCode {
    /// Creates a language diagnostic code.
    ///
    /// # Errors
    ///
    /// Returns [`LanguageCodeError`] when the code is empty or carries a
    /// character outside lowercase ASCII letters, digits and hyphens.
    pub fn new(code: impl Into<String>) -> Result<Self, LanguageCodeError> {
        let code = code.into();
        if code.is_empty() {
            return Err(LanguageCodeError::Empty);
        }
        if let Some(character) = code
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            return Err(LanguageCodeError::Character { code, character });
        }
        Ok(Self(code))
    }

    /// Returns the code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LanguageCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The diagnostics this crate raises, plus one open variant for the languages
/// built on it.
///
/// The set this crate owns is closed, because every one of its refusals is
/// spelled here and a consumer that branches on a code wants an exhaustive
/// match. The FHIRconnect and OMOCL crates raise their own codes through
/// [`DiagnosticCode::Language`], which carries a validated [`LanguageCode`]
/// rather than a type parameter: a parameter would travel through the loader,
/// the registry and every seam that carries a diagnostic, and force each
/// consumer to name the other language's code type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticCode {
    /// The file could not be read.
    Unreadable,
    /// The YAML could not be parsed.
    YamlSyntax,
    /// A YAML mapping wrote the same key twice.
    YamlDuplicateKey,
    /// A YAML alias names an anchor the document does not define.
    YamlUnresolvedAlias,
    /// The document holds no node at all.
    EmptyDocument,
    /// A node is not of the kind the header model needs there.
    UnexpectedNodeKind,
    /// A key the header model needs is absent.
    MissingKey,
    /// The `grammar` key names a language that is neither FHIRconnect nor
    /// OMOCL.
    UnknownGrammarLanguage,
    /// The `grammar` key carries a version outside the `vMAJOR.MINOR.PATCH`
    /// shape.
    MalformedGrammarVersion,
    /// The `type` key names a file type the mapping languages do not define.
    UnknownMappingType,
    /// The `metadata.name` value cannot serve as a unique mapping id.
    InvalidMappingName,
    /// The `spec.openEhrConfig.archetype` value is not an openEHR archetype
    /// id.
    InvalidArchetypeId,
    /// Two loaded files declare the same `metadata.name`.
    DuplicateMappingName,
    /// A mapping path is outside the grammar the two languages share.
    MalformedPath,
    /// A mapping path walks above the root of the anchor it resolves against.
    PathAboveAnchorRoot,
    /// A code raised by a language crate built on this foundation.
    Language(LanguageCode),
}

impl DiagnosticCode {
    /// Returns the stable spelling of the code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match *self {
            Self::Unreadable => "unreadable",
            Self::YamlSyntax => "yaml-syntax",
            Self::YamlDuplicateKey => "yaml-duplicate-key",
            Self::YamlUnresolvedAlias => "yaml-unresolved-alias",
            Self::EmptyDocument => "empty-document",
            Self::UnexpectedNodeKind => "unexpected-node-kind",
            Self::MissingKey => "missing-key",
            Self::UnknownGrammarLanguage => "unknown-grammar-language",
            Self::MalformedGrammarVersion => "malformed-grammar-version",
            Self::UnknownMappingType => "unknown-mapping-type",
            Self::InvalidMappingName => "invalid-mapping-name",
            Self::InvalidArchetypeId => "invalid-archetype-id",
            Self::DuplicateMappingName => "duplicate-mapping-name",
            Self::MalformedPath => "malformed-path",
            Self::PathAboveAnchorRoot => "path-above-anchor-root",
            Self::Language(ref code) => code.as_str(),
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One step of a path into the document model.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelPathSegment {
    /// A mapping key.
    Field(String),
    /// A 0-based sequence index, as the YAML document writes it.
    Index(usize),
}

impl fmt::Display for ModelPathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Field(ref name) => f.write_str(name),
            Self::Index(index) => write!(f, "[{index}]"),
        }
    }
}

/// A path into the document model, such as `mappings[3].with.openehr`.
///
/// This is a path through the loaded YAML, not an openEHR path. It names the
/// place in the file a diagnostic is about when the position alone would not
/// tell a reader which key is meant.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelPath(Vec<ModelPathSegment>);

impl ModelPath {
    /// Creates the path of the document root.
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Returns this path extended by one mapping key.
    #[must_use]
    pub fn field(&self, name: impl Into<String>) -> Self {
        let mut segments = self.0.clone();
        segments.push(ModelPathSegment::Field(name.into()));
        Self(segments)
    }

    /// Returns this path extended by one sequence index.
    #[must_use]
    pub fn index(&self, index: usize) -> Self {
        let mut segments = self.0.clone();
        segments.push(ModelPathSegment::Index(index));
        Self(segments)
    }

    /// Returns the segments of the path.
    #[must_use]
    pub fn segments(&self) -> &[ModelPathSegment] {
        &self.0
    }

    /// Whether this is the path of the document root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for ModelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for segment in &self.0 {
            match *segment {
                ModelPathSegment::Field(ref name) => {
                    if !first {
                        f.write_str(".")?;
                    }
                    f.write_str(name)?;
                }
                ModelPathSegment::Index(index) => write!(f, "[{index}]")?,
            }
            first = false;
        }
        Ok(())
    }
}

/// One refusal or remark about one place in one mapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    file: PathBuf,
    position: Option<Position>,
    mapping_name: Option<MappingName>,
    model_path: ModelPath,
    severity: Severity,
    code: DiagnosticCode,
    message: String,
}

impl Diagnostic {
    /// Creates a diagnostic of the given severity about the given file.
    #[must_use]
    pub fn new(
        severity: Severity,
        file: impl Into<PathBuf>,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file: file.into(),
            position: None,
            mapping_name: None,
            model_path: ModelPath::root(),
            severity,
            code,
            message: message.into(),
        }
    }

    /// Creates a diagnostic that refuses the file.
    #[must_use]
    pub fn error(
        file: impl Into<PathBuf>,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Error, file, code, message)
    }

    /// Creates a diagnostic that reports something about an accepted file.
    #[must_use]
    pub fn warning(
        file: impl Into<PathBuf>,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Warning, file, code, message)
    }

    /// Returns this diagnostic with the YAML position it is about.
    #[must_use]
    pub fn with_position(mut self, position: Position) -> Self {
        self.position = Some(position);
        self
    }

    /// Returns this diagnostic with the YAML position it is about, when one is
    /// known.
    #[must_use]
    pub fn with_optional_position(mut self, position: Option<Position>) -> Self {
        self.position = position;
        self
    }

    /// Returns this diagnostic with the name of the mapping it is about.
    #[must_use]
    pub fn with_mapping_name(mut self, name: MappingName) -> Self {
        self.mapping_name = Some(name);
        self
    }

    /// Returns this diagnostic with the model path it is about.
    #[must_use]
    pub fn with_model_path(mut self, model_path: ModelPath) -> Self {
        self.model_path = model_path;
        self
    }

    /// Returns the file the diagnostic is about.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Returns the YAML position the diagnostic is about, when one is known.
    #[must_use]
    pub const fn position(&self) -> Option<Position> {
        self.position
    }

    /// Returns the name of the mapping the diagnostic is about, when it is
    /// known.
    #[must_use]
    pub const fn mapping_name(&self) -> Option<&MappingName> {
        self.mapping_name.as_ref()
    }

    /// Returns the model path the diagnostic is about.
    #[must_use]
    pub const fn model_path(&self) -> &ModelPath {
        &self.model_path
    }

    /// Returns the severity.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the code.
    #[must_use]
    pub const fn code(&self) -> &DiagnosticCode {
        &self.code
    }

    /// Returns the message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Diagnostic {
    /// Renders `file:line:col: severity[code]: message (mapping name)`.
    ///
    /// The position is dropped when none is known, and the trailing
    /// parenthetical carries whichever of the mapping name and the model path
    /// are known, separated by a comma. It is dropped when neither is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file.display())?;
        if let Some(position) = self.position {
            write!(f, ":{position}")?;
        }
        write!(f, ": {}[{}]: {}", self.severity, self.code, self.message)?;
        match (self.mapping_name.as_ref(), self.model_path.is_root()) {
            (Some(name), true) => write!(f, " ({name})"),
            (Some(name), false) => write!(f, " ({name}, {})", self.model_path),
            (None, true) => Ok(()),
            (None, false) => write!(f, " ({})", self.model_path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Diagnostic;
    use super::DiagnosticCode;
    use super::LanguageCode;
    use super::LanguageCodeError;
    use super::ModelPath;
    use super::Severity;
    use crate::header::MappingName;
    use crate::position::Position;

    #[test]
    fn a_model_path_renders_fields_and_indices() {
        let path = ModelPath::root().field("mappings").index(3).field("with");
        assert_eq!(
            path.field("openehr").to_string(),
            "mappings[3].with.openehr"
        );
        assert!(ModelPath::root().is_root());
        assert_eq!(ModelPath::root().to_string(), "");
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn a_diagnostic_renders_file_position_severity_code_and_name()
    -> Result<(), Box<dyn core::error::Error>> {
        let diagnostic = Diagnostic::error(
            "model/observation.yml",
            DiagnosticCode::MissingKey,
            "the header has no `spec` key",
        )
        .with_position(Position::new(4, 1))
        .with_mapping_name(MappingName::new("OBSERVATION.body_weight.v2")?);
        assert_eq!(
            diagnostic.to_string(),
            "model/observation.yml:4:1: error[missing-key]: the header has no `spec` key \
             (OBSERVATION.body_weight.v2)"
        );
        Ok(())
    }

    #[test]
    fn a_diagnostic_without_a_position_renders_the_file_alone() {
        let diagnostic = Diagnostic::warning(
            "model/observation.yml",
            DiagnosticCode::Unreadable,
            "no such file",
        );
        assert_eq!(
            diagnostic.to_string(),
            "model/observation.yml: warning[unreadable]: no such file"
        );
        assert_eq!(diagnostic.severity(), Severity::Warning);
    }

    #[test]
    fn a_language_code_is_lowercase_letters_digits_and_hyphens() {
        let code = LanguageCode::new("followed-by-without-with").expect("a well-formed code");
        assert_eq!(
            DiagnosticCode::Language(code).to_string(),
            "followed-by-without-with"
        );
        assert_eq!(LanguageCode::new(""), Err(LanguageCodeError::Empty));
        assert!(matches!(
            LanguageCode::new("Bad_Code"),
            Err(LanguageCodeError::Character { .. })
        ));
    }
}
