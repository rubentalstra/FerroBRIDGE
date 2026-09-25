// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! JSON Schema validation of an OMOCL mapping file.
//!
//! OMOCL publishes no schema: its grammar is two railroad images and four
//! syntax tables (<https://github.com/SevKohler/OMOCL/wiki/Syntax-and-grammar>).
//! FerroBRIDGE authors one, `schemas/omocl-mapping.schema.json` beside the
//! crate source, closed at every level and carrying the per-target key sets of
//! [`crate::model::projection`]; the crate's tests assert the two agree and
//! that the schema admits every library file the lowering admits.
//!
//! YAML becomes JSON only here. A JSON Schema validator reads JSON, so the
//! positioned tree is projected into `serde_json::Value` for the validation
//! call and nothing downstream sees that projection.

use core::fmt;
use std::sync::LazyLock;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::schema::locate;
use openehr_mapping_core::schema::to_json;

use crate::model::error::ModelCode;
use crate::model::error::SchemaError;

/// The authored schema, as JSON text.
pub const SCHEMA: &str = include_str!("../../schemas/omocl-mapping.schema.json");

/// The compiled authored schema.
pub struct Schema {
    validator: jsonschema::Validator,
}

impl fmt::Debug for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Schema").finish_non_exhaustive()
    }
}

static COMPILED: LazyLock<Result<Schema, SchemaError>> = LazyLock::new(|| Schema::compile(SCHEMA));

/// Returns the compiled authored schema, compiling it on first use.
///
/// # Errors
///
/// Returns the compilation failure when the committed schema does not
/// compile, which `the_authored_schema_compiles` in this module pins.
pub fn schema() -> Result<&'static Schema, &'static SchemaError> {
    COMPILED.as_ref()
}

impl Schema {
    /// Compiles a schema from its JSON text, as draft-07, the draft the
    /// authored schema declares in its `$schema` keyword.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::Json`] when the document is not JSON and
    /// [`SchemaError::Compile`] when it is not a usable draft-07 schema.
    pub fn compile(source: &str) -> Result<Self, SchemaError> {
        let document: serde_json::Value =
            serde_json::from_str(source).map_err(|source| SchemaError::Json { source })?;
        let validator = jsonschema::draft7::options()
            .build(&document)
            .map_err(|source| SchemaError::Compile {
                source: Box::new(source),
            })?;
        Ok(Self { validator })
    }

    /// Returns the compiled validator.
    #[must_use]
    pub const fn validator(&self) -> &jsonschema::Validator {
        &self.validator
    }

    /// Validates one loaded document.
    ///
    /// The returned list is empty when the document validates, and otherwise
    /// carries one diagnostic per schema error, each positioned at the YAML
    /// node the error is about.
    #[must_use]
    pub fn validate_document(&self, document: &MappingDocument) -> Vec<Diagnostic> {
        let file = document.file().to_path_buf();
        let tree = document.document();
        let instance = match to_json(tree) {
            Ok(instance) => instance,
            Err(position) => {
                return vec![
                    Diagnostic::error(
                        file,
                        ModelCode::SchemaViolation.into(),
                        "the document holds a number JSON cannot represent, so it cannot be \
                         validated against a JSON Schema",
                    )
                    .with_position(position),
                ];
            }
        };
        let name = document.header().name().value().clone();
        self.validator
            .iter_errors(&instance)
            .map(|error| {
                let pointer = error.instance_path().to_string();
                let (path, position) = locate(tree, &pointer);
                Diagnostic::error(
                    file.clone(),
                    ModelCode::SchemaViolation.into(),
                    format!("omocl-mapping.schema.json: {error}"),
                )
                .with_position(position)
                .with_model_path(path)
                .with_mapping_name(name.clone())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::loader::load_str;
    use openehr_mapping_core::position::Position;

    use super::schema;
    use crate::model::error::ModelCode;
    use openehr_mapping_core::schema::locate;

    const FILE: &str = "grammar: OMOCL/v1.0.0\ntype: model\nmetadata:\n  name: Test_v1\n  \
                        version: 1.0.0\nspec:\n  system: OMOP\n  version: 5.4\n  \
                        openEhrConfig:\n    archetype: openEHR-EHR-OBSERVATION.test.v1\n\
                        mappings:\n  - type: \"Measurement\"\n    concept_id:\n      \
                        alternatives:\n        - code: 3004249\n";

    #[test]
    fn the_authored_schema_compiles() {
        assert!(schema().is_ok(), "the committed schema must compile");
    }

    #[test]
    fn a_well_formed_file_validates() {
        let document = load_str("test.yml", FILE).expect("a well-formed header");
        let schema = schema().expect("the schema compiles");
        let diagnostics = schema.validate_document(&document);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn an_unknown_column_key_is_refused_at_its_record() {
        let source = format!("{FILE}    colour:\n      alternatives:\n        - code: 1\n");
        let document = load_str("test.yml", &source).expect("a well-formed header");
        let schema = schema().expect("the schema compiles");
        let diagnostics = schema.validate_document(&document);
        assert!(!diagnostics.is_empty(), "the schema admitted `colour`");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::SchemaViolation.into());
        assert_eq!(diagnostic.model_path().to_string(), "mappings[0]");
    }

    #[test]
    fn a_pointer_resolves_to_the_position_of_its_node() {
        let document =
            openehr_mapping_core::loader::parse_str("test.yml", FILE).expect("well-formed YAML");
        let (path, position) = locate(&document, "/mappings/0/concept_id");
        assert_eq!(path.to_string(), "mappings[0].concept_id");
        assert_eq!(position, Position::new(13, 5));
    }
}
