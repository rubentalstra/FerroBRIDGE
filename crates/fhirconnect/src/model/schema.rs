// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! JSON Schema validation of a FHIRconnect mapping file, in two layers.
//!
//! [`published`] compiles the two schemas FHIRconnect ships
//! (`docs/specs/fhirconnect/modules/ROOT/pages/schema/schema.adoc`) from bytes
//! the caller supplies, so the vendored copies stay the one copy in the tree.
//! [`strict`] compiles the two schemas this crate ships beside its source,
//! which are closed at every level and carry the enums the prose fixes.
//!
//! YAML becomes JSON only here. A JSON Schema validator reads JSON, so the
//! positioned tree is projected into `serde_json::Value` for the validation
//! call and nothing downstream sees that projection; the model is built from
//! the positioned tree by [`crate::model::parse`].

use core::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::MappingType;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;

use crate::model::error::ModelCode;
use crate::model::error::SchemaError;
use crate::model::error::SchemaKind;

/// One compiled pair of FHIRconnect mapping schemas.
///
/// FHIRconnect publishes one schema for `model` and `extension` files and one
/// for `context` files, so a validator pair is the unit a caller works with.
pub struct Schemas {
    model: jsonschema::Validator,
    context: jsonschema::Validator,
}

impl fmt::Debug for Schemas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Schemas").finish_non_exhaustive()
    }
}

impl Schemas {
    /// Compiles a model schema and a context schema from their JSON text.
    ///
    /// Both are compiled as draft-07, the draft both published schemas
    /// declare in their `$schema` keyword.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::Json`] when either document is not JSON and
    /// [`SchemaError::Compile`] when either is not a usable draft-07 schema.
    pub fn compile(model: &str, context: &str) -> Result<Self, SchemaError> {
        Ok(Self {
            model: compile_one(model, SchemaKind::Model)?,
            context: compile_one(context, SchemaKind::Context)?,
        })
    }

    /// Returns the validator for `kind`.
    #[must_use]
    pub const fn validator(&self, kind: SchemaKind) -> &jsonschema::Validator {
        match kind {
            SchemaKind::Model => &self.model,
            SchemaKind::Context => &self.context,
        }
    }

    /// Validates one loaded document against the schema its header selects.
    ///
    /// A `model` or `extension` header selects the model schema and a
    /// `context` header the context schema
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/main.adoc`).
    /// A document whose header declares no `type` is validated against the
    /// model schema, which is what its `type` enum then refuses.
    #[must_use]
    pub fn validate_document(&self, document: &MappingDocument) -> Vec<Diagnostic> {
        let kind = match document.header().mapping_type().map(|t| *t.value()) {
            Some(MappingType::Context) => SchemaKind::Context,
            Some(MappingType::Model | MappingType::Extension) | None => SchemaKind::Model,
        };
        self.validate(document.file(), kind, document.document())
    }

    /// Validates one positioned document against the schema `kind` names.
    ///
    /// The returned list is empty when the document validates, and otherwise
    /// carries one diagnostic per schema error, each positioned at the YAML
    /// node the error is about.
    #[must_use]
    pub fn validate(
        &self,
        file: &Path,
        kind: SchemaKind,
        document: &PositionedValue,
    ) -> Vec<Diagnostic> {
        let file = file.to_path_buf();
        let instance = match to_json(document) {
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
        self.validator(kind)
            .iter_errors(&instance)
            .map(|error| {
                let pointer = error.instance_path().to_string();
                let (path, position) = locate(document, &pointer);
                Diagnostic::error(
                    file.clone(),
                    ModelCode::SchemaViolation.into(),
                    format!("{kind}: {error}"),
                )
                .with_position(position)
                .with_model_path(path)
            })
            .collect()
    }
}

/// Compiles one schema document as draft-07.
fn compile_one(source: &str, kind: SchemaKind) -> Result<jsonschema::Validator, SchemaError> {
    let document: serde_json::Value =
        serde_json::from_str(source).map_err(|source| SchemaError::Json {
            schema: kind,
            source,
        })?;
    jsonschema::draft7::options()
        .build(&document)
        .map_err(|source| SchemaError::Compile {
            schema: kind,
            source: Box::new(source),
        })
}

/// Projects a positioned YAML tree into JSON for the validation call.
///
/// Returns the position of the offending node when the tree holds a float
/// JSON cannot represent, which is the only value the projection can refuse.
fn to_json(node: &PositionedValue) -> Result<serde_json::Value, Position> {
    match *node.value() {
        MappingValue::Null => Ok(serde_json::Value::Null),
        MappingValue::Bool(value) => Ok(serde_json::Value::Bool(value)),
        MappingValue::Signed(value) => Ok(serde_json::Value::Number(value.into())),
        MappingValue::Unsigned(value) => Ok(serde_json::Value::Number(value.into())),
        MappingValue::Float(value) => serde_json::Number::from_f64(value.get())
            .map(serde_json::Value::Number)
            .ok_or_else(|| node.position()),
        MappingValue::Text(ref value) => Ok(serde_json::Value::String(value.clone())),
        MappingValue::Sequence(ref items) => items
            .iter()
            .map(to_json)
            .collect::<Result<Vec<serde_json::Value>, Position>>()
            .map(serde_json::Value::Array),
        MappingValue::Mapping(ref entries) => {
            let mut object = serde_json::Map::with_capacity(entries.len());
            for (key, entry) in entries {
                object.insert(key.clone(), to_json(entry.value())?);
            }
            Ok(serde_json::Value::Object(object))
        }
    }
}

/// Resolves a JSON Pointer against the positioned tree.
///
/// Returns the model path of the pointer and the position of the deepest node
/// it reaches, so a schema error about `/mappings/3/link` points at the `link`
/// key rather than at the document root. The pointer grammar is RFC 6901
/// (<https://www.rfc-editor.org/rfc/rfc6901>): `~1` is `/` and `~0` is `~`.
fn locate(root: &PositionedValue, pointer: &str) -> (ModelPath, Position) {
    let mut path = ModelPath::root();
    let mut node = root;
    let mut position = root.position();
    for raw in pointer.split('/').skip(1) {
        let token = raw.replace("~1", "/").replace("~0", "~");
        match node.value() {
            MappingValue::Mapping(_) => {
                let Some(entry) = node.entry(&token) else {
                    break;
                };
                path = path.field(&token);
                position = entry.key_position();
                node = entry.value();
            }
            MappingValue::Sequence(items) => {
                let Ok(index) = token.parse::<usize>() else {
                    break;
                };
                let Some(item) = items.get(index) else {
                    break;
                };
                path = path.index(index);
                position = item.position();
                node = item;
            }
            _ => break,
        }
    }
    (path, position)
}

/// The two schemas FHIRconnect publishes.
///
/// The published schemas are vendored under `docs/specs/fhirconnect/`, outside
/// this crate's packaged content, so the compiler here takes their bytes from
/// the caller. That keeps one copy of a vendored file in the tree
/// (`.claude/rules/vendored-inputs.md` forbids a second) and lets the corpus
/// test validate against the exact published bytes.
pub mod published {
    use crate::model::error::SchemaError;
    use crate::model::schema::Schemas;

    /// Compiles the published model and context schemas from their JSON text.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Schemas::compile`] returns.
    pub fn compile(model: &str, context: &str) -> Result<Schemas, SchemaError> {
        Schemas::compile(model, context)
    }
}

/// The two stricter schemas this crate ships.
///
/// They are closed at every level, carry the `operator`, `unidirectional`,
/// `extension` and data-type enums the prose fixes, type the `manual` items
/// and `hierarchy.split.openehr` the published schemas leave unvalidated, and
/// admit the four mapping keys the published model schema omits
/// (`mappingCode`, `link`, `participationsFunction` and mapping-level
/// `conceptmap`). `schemas/README.md` beside the crate source lists every
/// difference.
pub mod strict {
    use std::sync::LazyLock;

    use crate::model::error::SchemaError;
    use crate::model::schema::Schemas;

    /// The strict model and extension mapping schema, as JSON text.
    pub const MODEL_SCHEMA: &str = include_str!("../../schemas/model-mapping.schema.json");

    /// The strict context mapping schema, as JSON text.
    pub const CONTEXT_SCHEMA: &str = include_str!("../../schemas/contextual-mapping.schema.json");

    static COMPILED: LazyLock<Result<Schemas, SchemaError>> =
        LazyLock::new(|| Schemas::compile(MODEL_SCHEMA, CONTEXT_SCHEMA));

    /// Returns the compiled strict schemas, compiling them on first use.
    ///
    /// # Errors
    ///
    /// Returns the compilation failure when either committed schema does not
    /// compile, which `the_strict_schemas_compile` in this module pins.
    pub fn schemas() -> Result<&'static Schemas, &'static SchemaError> {
        COMPILED.as_ref()
    }
}

/// Holds the crate's own schema text so a caller can name the source file.
static STRICT_SOURCES: LazyLock<[(SchemaKind, &'static str); 2]> = LazyLock::new(|| {
    [
        (SchemaKind::Model, strict::MODEL_SCHEMA),
        (SchemaKind::Context, strict::CONTEXT_SCHEMA),
    ]
});

/// Returns the JSON text of the strict schema `kind` names.
#[must_use]
pub fn strict_source(kind: SchemaKind) -> &'static str {
    STRICT_SOURCES
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .map_or(strict::MODEL_SCHEMA, |(_, source)| *source)
}

/// Returns the path a vendored published schema is read from.
///
/// The corpus tests take the published schemas from the vendored tree rather
/// than from a copy inside this crate, and this is the shape of that path
/// relative to a repository root.
#[must_use]
pub fn vendored_published_path(kind: SchemaKind) -> PathBuf {
    PathBuf::from("docs/specs/fhirconnect/modules/ROOT/attachments").join(kind.file_name())
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::loader::load_str;
    use openehr_mapping_core::position::Position;

    use super::locate;
    use super::strict;
    use crate::model::error::ModelCode;
    use crate::model::error::SchemaKind;

    const MODEL: &str = "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: \
                         EVALUATION.test.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: \
                         R4\n  openEhrConfig:\n    archetype: \
                         openEHR-EHR-EVALUATION.test.v1\nmappings:\n  - name: \"a\"\n    with:\n  \
                         \x20   fhir: \"$resource.code\"\n      openehr: \"$archetype\"\n";

    #[test]
    fn the_strict_schemas_compile() {
        assert!(strict::schemas().is_ok(), "a committed schema must compile");
    }

    #[test]
    fn a_well_formed_model_file_validates() {
        let document = load_str("model.yml", MODEL).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn a_mapping_key_the_grammar_does_not_define_is_refused() {
        let source = format!("{MODEL}    hardcodedValue: \"x\"\n");
        let document = load_str("model.yml", &source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::SchemaViolation.into());
        assert_eq!(diagnostic.model_path().to_string(), "mappings[0]");
    }

    #[test]
    fn the_four_omitted_concept_keys_validate() {
        let source = "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: \
                      EVALUATION.test.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: \
                      R4\nmappings:\n  - name: \"programmed\"\n    mappingCode: \"toDaily\"\n  - \
                      name: \"linked\"\n    link:\n      meaning: \"the case\"\n      type: \
                      \"case\"\n  - name: \"participation\"\n    participationsFunction: \
                      \"asserter\"\n  - name: \"translated\"\n    conceptmap: \
                      \"http://example.invalid/ConceptMap/a\"\n";
        let document = load_str("model.yml", source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn a_null_mappings_is_refused_by_the_strict_schema() {
        let source = "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: \
                      EVALUATION.test.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: \
                      R4\nmappings:\n";
        let document = load_str("model.yml", source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn a_root_unidirectional_is_refused_by_the_strict_schema() {
        let source = "grammar: FHIRConnect/v1.0.0\ntype: model\nunidirectional: \
                      \"openehr->fhir\"\nmetadata:\n  name: EVALUATION.test.v1\n  version: \
                      1.0.0\nspec:\n  system: FHIR\n  version: R4\n";
        let document = load_str("model.yml", source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn both_documented_direction_spellings_and_the_library_casing_validate() {
        for value in [
            "openehr->fhir",
            "fhir->openehr",
            "openEHR->fhir",
            "fhir->openEHR",
        ] {
            let source = format!("{MODEL}    unidirectional: \"{value}\"\n");
            let document = load_str("model.yml", &source).expect("a well-formed header");
            let schemas = strict::schemas().expect("the strict schemas compile");
            let diagnostics = schemas.validate_document(&document);
            assert!(diagnostics.is_empty(), "{value}: {diagnostics:?}");
        }
    }

    #[test]
    fn a_direction_outside_the_set_is_refused_by_the_strict_schema() {
        let source = format!("{MODEL}    unidirectional: \"openehr<->fhir\"\n");
        let document = load_str("model.yml", &source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn an_openehr_condition_naming_no_attribute_is_refused_by_the_strict_schema() {
        let source = format!(
            "{MODEL}    openehrCondition:\n      targetRoot: \"$archetype\"\n      operator: \"not empty\"\n"
        );
        let document = load_str("model.yml", &source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::SchemaViolation.into());
    }

    #[test]
    fn either_openehr_condition_attribute_spelling_validates() {
        for attribute in [
            "      targetAttribute: \"items[at0071]\"\n",
            "      targetAttributes:\n        - \"items[at0071]\"\n",
        ] {
            let source = format!(
                "{MODEL}    openehrCondition:\n      targetRoot: \"$archetype\"\n      operator: \"not empty\"\n{attribute}"
            );
            let document = load_str("model.yml", &source).expect("a well-formed header");
            let schemas = strict::schemas().expect("the strict schemas compile");
            let diagnostics = schemas.validate_document(&document);
            assert!(diagnostics.is_empty(), "{attribute}: {diagnostics:?}");
        }
    }

    #[test]
    fn a_fhir_condition_naming_no_attribute_validates() {
        let source = format!(
            "{MODEL}    fhirCondition:\n      targetRoot: \"$resource.code\"\n      operator: \"not empty\"\n"
        );
        let document = load_str("model.yml", &source).expect("a well-formed header");
        let schemas = strict::schemas().expect("the strict schemas compile");
        let diagnostics = schemas.validate_document(&document);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn a_pointer_resolves_to_the_position_of_its_node() {
        let document =
            openehr_mapping_core::loader::parse_str("model.yml", MODEL).expect("well-formed YAML");
        let (path, position) = locate(&document, "/mappings/0/with/openehr");
        assert_eq!(path.to_string(), "mappings[0].with.openehr");
        assert_eq!(position, Position::new(15, 7));
    }

    #[test]
    fn the_vendored_published_path_names_the_published_file() {
        let path = super::vendored_published_path(SchemaKind::Context);
        assert!(
            path.ends_with("contextual-mapping.schema.json"),
            "{}",
            path.display()
        );
        assert_eq!(
            super::strict_source(SchemaKind::Context),
            strict::CONTEXT_SCHEMA
        );
    }
}
