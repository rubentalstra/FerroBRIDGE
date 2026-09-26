// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowering from a positioned YAML tree to the FHIRconnect file model.
//!
//! The loader in `openehr-mapping-core` parses the YAML, resolves anchors and
//! aliases and reads the shared header; this module reads the FHIRconnect
//! grammar out of the tree it returns. The lowering is a validator in its own
//! right: it refuses a key the grammar does not define, a node of the wrong
//! kind and a keyword value outside its documented set, and it collects every
//! refusal rather than stopping at the first, so one pass reports the whole
//! file.

pub mod mapping;
pub mod spec;

use core::str::FromStr;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::Header;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::header::metadata::MappingType;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::ContextMappingFile;
use crate::model::ast::ModelMappingFile;
use crate::model::ast::keyword::Variable;
use crate::model::error::ModelCode;

use crate::model::parse::mapping::lower_mappings;
use crate::model::parse::spec::lower_context_block;
use crate::model::parse::spec::lower_preprocessor;
use crate::model::parse::spec::lower_spec;

/// The keys a model or extension mapping file writes at the document root.
const ROOT_KEYS: &[&str] = &[
    "grammar",
    "type",
    "metadata",
    "spec",
    "mappings",
    "preprocessor",
];

/// The keys a context mapping file writes at the document root.
const CONTEXT_ROOT_KEYS: &[&str] = &["grammar", "type", "metadata", "spec", "context"];

/// The keys the `spec` block of a model or extension mapping file writes.
const SPEC_KEYS: &[&str] = &[
    "system",
    "version",
    "extends",
    "unidirectional",
    "conceptmap",
    "openEhrConfig",
    "fhirConfig",
];

/// The keys the `spec` block of a context mapping file writes.
const CONTEXT_SPEC_KEYS: &[&str] = &["system", "version"];

/// The keys `spec.openEhrConfig` writes.
const OPENEHR_CONFIG_KEYS: &[&str] = &["archetype", "revision"];

/// The keys `spec.fhirConfig` writes.
const FHIR_CONFIG_KEYS: &[&str] = &["structureDefinition"];

/// The keys one mapping method writes.
const MAPPING_KEYS: &[&str] = &[
    "name",
    "type",
    "extension",
    "appendTo",
    "with",
    "unidirectional",
    "manual",
    "fhirCondition",
    "openehrCondition",
    "followedBy",
    "reference",
    "slotArchetype",
    "mappingCode",
    "link",
    "participationsFunction",
    "conceptmap",
];

/// The keys a `with` block writes.
const WITH_KEYS: &[&str] = &["fhir", "openehr", "type", "value"];

/// The keys a condition writes.
const CONDITION_KEYS: &[&str] = &[
    "targetRoot",
    "targetAttribute",
    "targetAttributes",
    "operator",
    "criteria",
    "criterias",
    "identifying",
];

/// The keys one `manual` entry writes.
const MANUAL_KEYS: &[&str] = &[
    "name",
    "fhir",
    "openehr",
    "fhirCondition",
    "openehrCondition",
    "value",
    "unidirectional",
];

/// The keys one `path` and `value` pair of a manual entry writes.
const MANUAL_PATH_KEYS: &[&str] = &["path", "value"];

/// The keys a `link` block writes.
const LINK_KEYS: &[&str] = &["meaning", "type"];

/// The keys a `preprocessor` block writes.
const PREPROCESSOR_KEYS: &[&str] = &["fhirCondition", "openehrCondition", "hierarchy"];

/// The keys a `hierarchy` block writes.
const HIERARCHY_KEYS: &[&str] = &["with", "split"];

/// The keys a `hierarchy.split` block writes.
const SPLIT_KEYS: &[&str] = &["fhir", "openehr"];

/// The keys one side of a `hierarchy.split` writes.
const SPLIT_TARGET_KEYS: &[&str] = &["create", "path", "unique"];

/// The keys a `context` block writes.
const CONTEXT_KEYS: &[&str] = &[
    "profile",
    "template",
    "archetypes",
    "extensions",
    "operational",
    "start",
];

/// The keys `context.profile` writes.
const PROFILE_KEYS: &[&str] = &["url", "version"];

/// The keys `context.template` writes.
const TEMPLATE_KEYS: &[&str] = &["id", "sem_ver"];

/// The collector one file's lowering runs against.
///
/// It owns the file name every diagnostic carries and the growing list of
/// refusals, so a lowering step can report and carry on.
#[derive(Debug)]
struct Lowering {
    file: PathBuf,
    mapping_name: Option<MappingName>,
    diagnostics: Vec<Diagnostic>,
}

impl Lowering {
    /// Starts a lowering of `file`.
    fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            mapping_name: None,
            diagnostics: Vec::new(),
        }
    }

    /// Records one refusal.
    fn report(
        &mut self,
        code: ModelCode,
        position: Position,
        path: &ModelPath,
        message: impl Into<String>,
    ) {
        let mut diagnostic = Diagnostic::error(self.file.clone(), code.into(), message)
            .with_position(position)
            .with_model_path(path.clone());
        if let Some(ref name) = self.mapping_name {
            diagnostic = diagnostic.with_mapping_name(name.clone());
        }
        self.diagnostics.push(diagnostic);
    }

    /// Records one refusal under a diagnostic code the shared foundation owns.
    fn report_core(
        &mut self,
        code: DiagnosticCode,
        position: Position,
        path: &ModelPath,
        message: impl Into<String>,
    ) {
        let mut diagnostic = Diagnostic::error(self.file.clone(), code, message)
            .with_position(position)
            .with_model_path(path.clone());
        if let Some(ref name) = self.mapping_name {
            diagnostic = diagnostic.with_mapping_name(name.clone());
        }
        self.diagnostics.push(diagnostic);
    }

    /// Refuses every key of `node` that is not in `allowed`.
    fn refuse_unknown_keys(&mut self, node: &PositionedValue, path: &ModelPath, allowed: &[&str]) {
        let Some(entries) = node.as_mapping() else {
            return;
        };
        for (key, entry) in entries {
            if allowed.contains(&key.as_str()) {
                continue;
            }
            let code = if path.is_root() && key == "unidirectional" {
                ModelCode::RootUnidirectional
            } else {
                ModelCode::UnknownKey
            };
            let message = match code {
                ModelCode::RootUnidirectional => format!(
                    "`{key}` is not a document-root key; the published \
                     model-mapping.schema.json defines `unidirectional` under `spec` only"
                ),
                _ => format!("`{key}` is not a key the FHIRconnect grammar defines here"),
            };
            self.report(code, entry.key_position(), &path.field(key), message);
        }
    }

    /// Reads a node that must be a mapping.
    fn mapping<'a>(
        &mut self,
        node: &'a PositionedValue,
        path: &ModelPath,
    ) -> Option<&'a PositionedValue> {
        if node.as_mapping().is_some() {
            return Some(node);
        }
        self.wrong_kind(node, path, ValueKind::Mapping);
        None
    }

    /// Reads a node that must be a string.
    fn text(&mut self, node: &PositionedValue, path: &ModelPath) -> Option<Located<String>> {
        if let Some(text) = node.as_text() {
            return Some(Located::new(node.position(), text.to_owned()));
        }
        self.wrong_kind(node, path, ValueKind::Text);
        None
    }

    /// Reads a node that must be a boolean.
    fn boolean(&mut self, node: &PositionedValue, path: &ModelPath) -> Option<Located<bool>> {
        if let MappingValue::Bool(value) = *node.value() {
            return Some(Located::new(node.position(), value));
        }
        self.wrong_kind(node, path, ValueKind::Bool);
        None
    }

    /// Records a node whose kind is not the one the grammar needs.
    fn wrong_kind(&mut self, node: &PositionedValue, path: &ModelPath, expected: ValueKind) {
        let message = format!("expected {expected} at `{path}`, found {}", node.kind());
        self.report(
            ModelCode::UnexpectedNodeKind,
            node.position(),
            path,
            message,
        );
    }

    /// Records a key the grammar needs and the file does not write.
    fn missing_key(&mut self, node: &PositionedValue, path: &ModelPath, key: &str) {
        let child = path.field(key);
        let message = format!("the FHIRconnect grammar needs a `{key}` key here");
        self.report(ModelCode::MissingKey, node.position(), &child, message);
    }

    /// Reads a required string under `key`.
    fn required_text(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        key: &str,
    ) -> Option<Located<String>> {
        if let Some(value) = node.get(key) {
            return self.text(value, &path.field(key));
        }
        self.missing_key(node, path, key);
        None
    }

    /// Reads an optional string under `key`.
    fn optional_text(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        key: &str,
    ) -> Option<Located<String>> {
        let value = node.get(key)?;
        self.text(value, &path.field(key))
    }

    /// Reads an optional keyword value under `key` through its `FromStr`.
    fn optional_keyword<T>(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        key: &str,
        code: ModelCode,
    ) -> Option<Located<T>>
    where
        T: FromStr<Err = crate::model::ast::keyword::KeywordError>,
    {
        let child = path.field(key);
        let text = self.optional_text(node, path, key)?;
        match T::from_str(text.value()) {
            Ok(parsed) => Some(Located::new(text.position(), parsed)),
            Err(error) => {
                self.report(code, text.position(), &child, error.to_string());
                None
            }
        }
    }

    /// Reads an optional mapping name under `key`, treating a null as absent.
    fn optional_mapping_name(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        key: &str,
    ) -> Option<Located<MappingName>> {
        let value = node.get(key)?;
        if matches!(*value.value(), MappingValue::Null) {
            return None;
        }
        let text = self.text(value, &path.field(key))?;
        self.mapping_name(&text, &path.field(key))
    }

    /// Turns a located string into a mapping name.
    fn mapping_name(
        &mut self,
        text: &Located<String>,
        path: &ModelPath,
    ) -> Option<Located<MappingName>> {
        match MappingName::new(text.value().clone()) {
            Ok(name) => Some(Located::new(text.position(), name)),
            Err(error) => {
                self.report_core(
                    DiagnosticCode::InvalidMappingName,
                    text.position(),
                    path,
                    error.to_string(),
                );
                None
            }
        }
    }

    /// Reads a sequence of strings under `key`, accepting a lone scalar as a
    /// one-element sequence.
    ///
    /// The specification writes both forms: "`criteria` can be either a single
    /// element, or as a list (using `-`)"
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
    /// §criteria).
    fn text_list(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        key: &str,
    ) -> Vec<Located<String>> {
        let child = path.field(key);
        let Some(value) = node.get(key) else {
            return Vec::new();
        };
        match value.as_sequence() {
            Some(items) => items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| self.text(item, &child.index(index)))
                .collect(),
            None => self.text(value, &child).into_iter().collect(),
        }
    }

    /// Refuses a path whose head variable is outside the documented set.
    ///
    /// A `criteria` or a `manual` `value` is a literal rather than a path, so
    /// neither is checked here even when it opens with a `$`; the one
    /// documented `$context.who` value form
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`,
    /// §`$context`) is read by the interpreter.
    fn check_path_variable(&mut self, text: &Located<String>, path: &ModelPath) {
        let value = text.value();
        if !value.starts_with('$') {
            return;
        }
        let head = value
            .split(['/', '.', '[', '(', ')'])
            .next()
            .unwrap_or(value.as_str());
        if let Err(error) = Variable::from_str(head) {
            self.report(
                ModelCode::UnknownPathVariable,
                text.position(),
                path,
                error.to_string(),
            );
        }
    }

    /// Reads a path-valued string under `key` and checks its head variable.
    fn optional_path(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        key: &str,
    ) -> Option<Located<String>> {
        let text = self.optional_text(node, path, key)?;
        self.check_path_variable(&text, &path.field(key));
        Some(text)
    }
}

/// Reads the FHIRconnect grammar out of a loaded model or extension file.
///
/// # Errors
///
/// Returns every refusal the lowering raised, each carrying the file, the
/// YAML position and the path into the document model. The list is never
/// empty on the error side and never truncated to the first refusal.
pub fn lower_model(document: &MappingDocument) -> Result<ModelMappingFile, Vec<Diagnostic>> {
    let mut lowering = Lowering::new(document.file().to_path_buf());
    let root = ModelPath::root();
    let node = document.document();
    let header = document.header();
    require_file_type(
        &mut lowering,
        header,
        &[MappingType::Model, MappingType::Extension],
    );
    if lowering.mapping(node, &root).is_none() {
        return Err(lowering.diagnostics);
    }
    lowering.refuse_unknown_keys(node, &root, ROOT_KEYS);

    let spec = lower_spec(&mut lowering, node, &root);
    let preprocessor = lower_preprocessor(&mut lowering, node, &root);
    let mappings = lower_mappings(&mut lowering, node, &root);

    match spec {
        Some(spec) if lowering.diagnostics.is_empty() => Ok(ModelMappingFile::new(
            document.file().to_path_buf(),
            header.clone(),
            spec,
            preprocessor,
            mappings,
        )),
        _ => Err(lowering.diagnostics),
    }
}

/// Reads the FHIRconnect grammar out of a loaded context file.
///
/// # Errors
///
/// Returns every refusal the lowering raised, as [`lower_model`] does.
pub fn lower_context(document: &MappingDocument) -> Result<ContextMappingFile, Vec<Diagnostic>> {
    let mut lowering = Lowering::new(document.file().to_path_buf());
    let root = ModelPath::root();
    let node = document.document();
    let header = document.header();
    require_file_type(&mut lowering, header, &[MappingType::Context]);
    if lowering.mapping(node, &root).is_none() {
        return Err(lowering.diagnostics);
    }
    lowering.refuse_unknown_keys(node, &root, CONTEXT_ROOT_KEYS);

    if let Some(spec) = node.get("spec") {
        let spec_path = root.field("spec");
        if lowering.mapping(spec, &spec_path).is_some() {
            lowering.refuse_unknown_keys(spec, &spec_path, CONTEXT_SPEC_KEYS);
            let _system = lowering.required_text(spec, &spec_path, "system");
            let _version = lowering.required_text(spec, &spec_path, "version");
        }
    }

    let context = lower_context_block(&mut lowering, node, &root);

    match context {
        Some(context) if lowering.diagnostics.is_empty() => Ok(ContextMappingFile::new(
            document.file().to_path_buf(),
            header.clone(),
            context,
        )),
        _ => Err(lowering.diagnostics),
    }
}

/// Refuses a file whose `type` is not one the entry point loads.
fn require_file_type(lowering: &mut Lowering, header: &Header, admitted: &[MappingType]) {
    lowering.mapping_name = Some(header.name().value().clone());
    let Some(declared) = header.mapping_type() else {
        return;
    };
    if admitted.contains(declared.value()) {
        return;
    }
    let admitted = admitted
        .iter()
        .map(|kind| format!("`{kind}`"))
        .collect::<Vec<String>>()
        .join(" or ");
    let message = format!(
        "this entry point loads a {admitted} mapping file, and the header declares `{}`",
        declared.value()
    );
    lowering.report(
        ModelCode::WrongMappingFileType,
        declared.position(),
        &ModelPath::root().field("type"),
        message,
    );
}
