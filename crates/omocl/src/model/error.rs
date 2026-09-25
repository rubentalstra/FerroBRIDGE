// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The diagnostic codes the OMOCL model layer raises.
//!
//! Every refusal in this module tree renders into the one
//! [`openehr_mapping_core::diagnostic::Diagnostic`] the shared foundation
//! defines, carrying the file, the YAML position, the mapping name and the
//! path into the document model. The codes below are the OMOCL half of that
//! vocabulary and travel as
//! [`openehr_mapping_core::diagnostic::DiagnosticCode::Language`].

use core::fmt;

use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::LanguageCode;

/// A refusal the OMOCL model layer raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ModelCode {
    /// A key the grammar does not define appears in the file.
    UnknownKey,
    /// A key the corpus writes names no column of the CDM target it is under.
    KeyWithoutColumn,
    /// A node is not of the kind the grammar needs there.
    UnexpectedNodeKind,
    /// A key the grammar needs is absent.
    MissingKey,
    /// The `grammar` names a grammar version this crate does not implement.
    UnsupportedGrammar,
    /// The header `type` is not `model`.
    UnsupportedFileType,
    /// `spec.system` or `spec.version` names something other than OMOP CDM
    /// v5.4.
    UnsupportedTarget,
    /// A record `type` is none of the twelve the grammar admits.
    UnknownType,
    /// `mappings` or `alternatives` holds no item.
    EmptyList,
    /// An alternative writes none of `path`, `code`, `conceptMap` and
    /// `multiplication`.
    EmptyAlternative,
    /// An alternative writes more than one of `path`, `code`, `conceptMap` and
    /// `multiplication`.
    AmbiguousAlternative,
    /// A literal concept id is not a CDM `integer`.
    InvalidConceptId,
    /// A literal `multiplication` factor does not fit a 64-bit signed integer.
    InvalidFactor,
    /// A `conceptMap` key is not an archetype at-code.
    InvalidAtCode,
    /// Two keys of one record project onto the same CDM column.
    ColumnClaimedTwice,
    /// A `CustomMapping` names no registered converter.
    UnknownConverter,
    /// An `Include` names an archetype no loaded file maps.
    UnresolvedInclude,
    /// A literal concept id is absent from the loaded vocabulary.
    UnknownConcept,
    /// A literal concept id belongs to a domain its CDM column does not admit.
    DomainMismatch,
    /// A mapping file does not validate against the authored JSON Schema.
    SchemaViolation,
    /// The authored schema does not compile.
    SchemaCompilation,
}

impl ModelCode {
    /// Returns the stable spelling of the code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownKey => "omocl-unknown-key",
            Self::KeyWithoutColumn => "omocl-key-without-column",
            Self::UnexpectedNodeKind => "omocl-unexpected-node-kind",
            Self::MissingKey => "omocl-missing-key",
            Self::UnsupportedGrammar => "omocl-unsupported-grammar",
            Self::UnsupportedFileType => "omocl-unsupported-file-type",
            Self::UnsupportedTarget => "omocl-unsupported-target",
            Self::UnknownType => "omocl-unknown-type",
            Self::EmptyList => "omocl-empty-list",
            Self::EmptyAlternative => "omocl-empty-alternative",
            Self::AmbiguousAlternative => "omocl-ambiguous-alternative",
            Self::InvalidConceptId => "omocl-invalid-concept-id",
            Self::InvalidFactor => "omocl-invalid-factor",
            Self::InvalidAtCode => "omocl-invalid-at-code",
            Self::ColumnClaimedTwice => "omocl-column-claimed-twice",
            Self::UnknownConverter => "omocl-unknown-converter",
            Self::UnresolvedInclude => "omocl-unresolved-include",
            Self::UnknownConcept => "omocl-unknown-concept",
            Self::DomainMismatch => "omocl-domain-mismatch",
            Self::SchemaViolation => "omocl-schema-violation",
            Self::SchemaCompilation => "omocl-schema-compilation",
        }
    }

    /// Every code this enum defines, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::UnknownKey,
            Self::KeyWithoutColumn,
            Self::UnexpectedNodeKind,
            Self::MissingKey,
            Self::UnsupportedGrammar,
            Self::UnsupportedFileType,
            Self::UnsupportedTarget,
            Self::UnknownType,
            Self::EmptyList,
            Self::EmptyAlternative,
            Self::AmbiguousAlternative,
            Self::InvalidConceptId,
            Self::InvalidFactor,
            Self::InvalidAtCode,
            Self::ColumnClaimedTwice,
            Self::UnknownConverter,
            Self::UnresolvedInclude,
            Self::UnknownConcept,
            Self::DomainMismatch,
            Self::SchemaViolation,
            Self::SchemaCompilation,
        ]
    }
}

impl fmt::Display for ModelCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ModelCode> for DiagnosticCode {
    /// # Panics
    ///
    /// Never in practice. Every spelling [`ModelCode::as_str`] returns is
    /// lowercase ASCII letters and hyphens, which is the alphabet
    /// [`LanguageCode::new`] admits, and `every_code_is_a_language_code` in
    /// this module asserts that over [`ModelCode::all`].
    #[expect(
        clippy::expect_used,
        reason = "the code alphabet is fixed by as_str and pinned by every_code_is_a_language_code"
    )]
    fn from(code: ModelCode) -> Self {
        let language =
            LanguageCode::new(code.as_str()).expect("a model code should be a language code");
        Self::Language(language)
    }
}

/// Why the authored JSON Schema could not be compiled into a validator.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// The schema document is not JSON.
    #[error("the OMOCL mapping schema is not JSON")]
    Json {
        /// The underlying failure.
        #[source]
        source: serde_json::Error,
    },
    /// The schema document is JSON but is not a usable JSON Schema.
    #[error("the OMOCL mapping schema does not compile")]
    Compile {
        /// The underlying failure.
        #[source]
        source: Box<jsonschema::ValidationError<'static>>,
    },
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::diagnostic::DiagnosticCode;
    use openehr_mapping_core::diagnostic::LanguageCode;

    use super::ModelCode;

    #[test]
    fn every_code_is_a_language_code() {
        for code in ModelCode::all() {
            assert!(
                LanguageCode::new(code.as_str()).is_ok(),
                "the code `{code}` is outside the language-code alphabet"
            );
        }
    }

    #[test]
    fn every_code_has_a_distinct_spelling() {
        let mut spellings: Vec<&str> = ModelCode::all().iter().map(|c| c.as_str()).collect();
        let total = spellings.len();
        spellings.sort_unstable();
        spellings.dedup();
        assert_eq!(spellings.len(), total, "two model codes share a spelling");
    }

    #[test]
    fn a_code_renders_as_its_diagnostic_code() {
        assert_eq!(
            DiagnosticCode::from(ModelCode::UnknownKey).to_string(),
            "omocl-unknown-key"
        );
    }
}
