// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The diagnostic codes the FHIRconnect model layer raises.
//!
//! Every refusal in this module tree renders into the one
//! [`openehr_mapping_core::diagnostic::Diagnostic`] the shared foundation
//! defines, carrying the file, the YAML position, the mapping name and the
//! path into the document model. The codes below are the FHIRconnect half of
//! that vocabulary and travel as
//! [`openehr_mapping_core::diagnostic::DiagnosticCode::Language`].

use core::fmt;

use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::LanguageCode;

/// A refusal the FHIRconnect model layer raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelCode {
    /// A key the FHIRconnect grammar does not define appears in the file.
    UnknownKey,
    /// A node is not of the kind the grammar needs there.
    UnexpectedNodeKind,
    /// A key the grammar needs is absent.
    MissingKey,
    /// The file's `type` is not the one the entry point was asked to load.
    WrongMappingFileType,
    /// A `unidirectional` value is outside the two documented spellings.
    InvalidDirection,
    /// An `extension` value is outside `add`, `append` and `overwrite`.
    InvalidExtensionMethod,
    /// A `with.type` value is outside the data-type enum.
    InvalidDataType,
    /// An `operator` value is outside the five documented operators.
    InvalidOperator,
    /// A `$name` variable is outside the seven documented variables.
    UnknownPathVariable,
    /// The file writes `unidirectional` at the document root.
    RootUnidirectional,
    /// `mappings` is written with no value.
    NullMappings,
    /// A mapping file does not validate against a JSON Schema.
    SchemaViolation,
    /// A schema document does not compile.
    SchemaCompilation,
    /// A condition's `targetRoot` does not align with the `with` path it
    /// filters.
    ConditionTargetRootMismatch,
    /// A condition writes `criteria` under an operator that takes none.
    CriteriaNotAllowed,
    /// A condition omits `criteria` under an operator that needs one.
    CriteriaMissing,
    /// A cross-file reference names no loaded `metadata.name`.
    UnknownMappingReference,
    /// An extension method appears in a file that is not `type: extension`.
    ExtensionMethodOutsideExtensionFile,
    /// A `reference` mapping does not write `openehr: "$reference"`.
    ReferenceWithoutReferenceVariable,
    /// A `mappingCode` names no registered function.
    UnknownMappingCode,
}

impl ModelCode {
    /// Returns the stable spelling of the code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownKey => "fc-unknown-key",
            Self::UnexpectedNodeKind => "fc-unexpected-node-kind",
            Self::MissingKey => "fc-missing-key",
            Self::WrongMappingFileType => "fc-wrong-mapping-file-type",
            Self::InvalidDirection => "fc-invalid-direction",
            Self::InvalidExtensionMethod => "fc-invalid-extension-method",
            Self::InvalidDataType => "fc-invalid-data-type",
            Self::InvalidOperator => "fc-invalid-operator",
            Self::UnknownPathVariable => "fc-unknown-path-variable",
            Self::RootUnidirectional => "fc-root-unidirectional",
            Self::NullMappings => "fc-null-mappings",
            Self::SchemaViolation => "fc-schema-violation",
            Self::SchemaCompilation => "fc-schema-compilation",
            Self::ConditionTargetRootMismatch => "fc-condition-target-root-mismatch",
            Self::CriteriaNotAllowed => "fc-criteria-not-allowed",
            Self::CriteriaMissing => "fc-criteria-missing",
            Self::UnknownMappingReference => "fc-unknown-mapping-reference",
            Self::ExtensionMethodOutsideExtensionFile => "fc-extension-method-outside-model",
            Self::ReferenceWithoutReferenceVariable => "fc-reference-without-reference-variable",
            Self::UnknownMappingCode => "fc-unknown-mapping-code",
        }
    }

    /// Every code this enum defines, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::UnknownKey,
            Self::UnexpectedNodeKind,
            Self::MissingKey,
            Self::WrongMappingFileType,
            Self::InvalidDirection,
            Self::InvalidExtensionMethod,
            Self::InvalidDataType,
            Self::InvalidOperator,
            Self::UnknownPathVariable,
            Self::RootUnidirectional,
            Self::NullMappings,
            Self::SchemaViolation,
            Self::SchemaCompilation,
            Self::ConditionTargetRootMismatch,
            Self::CriteriaNotAllowed,
            Self::CriteriaMissing,
            Self::UnknownMappingReference,
            Self::ExtensionMethodOutsideExtensionFile,
            Self::ReferenceWithoutReferenceVariable,
            Self::UnknownMappingCode,
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

/// Why a JSON Schema document could not be compiled into a validator.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The schema document is not JSON.
    #[error("the {schema} schema is not JSON")]
    Json {
        /// Which schema failed.
        schema: SchemaKind,
        /// The underlying failure.
        #[source]
        source: serde_json::Error,
    },
    /// The schema document is JSON but is not a usable JSON Schema.
    #[error("the {schema} schema does not compile")]
    Compile {
        /// Which schema failed.
        schema: SchemaKind,
        /// The underlying failure.
        #[source]
        source: Box<jsonschema::ValidationError<'static>>,
    },
}

/// Which of the two FHIRconnect mapping schemas is in play.
///
/// FHIRconnect publishes one schema per file family: `model-mapping.schema.json`
/// covers `type: model` and `type: extension`, and
/// `contextual-mapping.schema.json` covers `type: context`
/// (`docs/specs/fhirconnect/modules/ROOT/pages/schema/schema.adoc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaKind {
    /// The model and extension schema.
    Model,
    /// The context schema.
    Context,
}

impl SchemaKind {
    /// Returns the published file name of the schema.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Model => "model-mapping.schema.json",
            Self::Context => "contextual-mapping.schema.json",
        }
    }
}

impl fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_name())
    }
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::diagnostic::DiagnosticCode;
    use openehr_mapping_core::diagnostic::LanguageCode;

    use super::ModelCode;
    use super::SchemaKind;

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
            DiagnosticCode::from(ModelCode::NullMappings).to_string(),
            "fc-null-mappings"
        );
    }

    #[test]
    fn a_schema_kind_names_its_published_file() {
        assert_eq!(SchemaKind::Model.to_string(), "model-mapping.schema.json");
        assert_eq!(
            SchemaKind::Context.to_string(),
            "contextual-mapping.schema.json"
        );
    }
}
