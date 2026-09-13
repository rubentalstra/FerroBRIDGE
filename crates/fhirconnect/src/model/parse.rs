// SPDX-FileCopyrightText: Ruben Talstra
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

use core::str::FromStr;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::Header;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::header::MappingType;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::Condition;
use crate::model::ast::ConditionOperator;
use crate::model::ast::ContextMappingFile;
use crate::model::ast::DataType;
use crate::model::ast::Direction;
use crate::model::ast::ExtensionMethod;
use crate::model::ast::FollowedBy;
use crate::model::ast::Hierarchy;
use crate::model::ast::HierarchySplit;
use crate::model::ast::LinkMapping;
use crate::model::ast::ManualEntry;
use crate::model::ast::ManualPath;
use crate::model::ast::Mapping;
use crate::model::ast::MappingContext;
use crate::model::ast::ModelMappingFile;
use crate::model::ast::ModelSpec;
use crate::model::ast::Preprocessor;
use crate::model::ast::ProfileRef;
use crate::model::ast::ReferenceMapping;
use crate::model::ast::SplitTarget;
use crate::model::ast::TemplateRef;
use crate::model::ast::Variable;
use crate::model::ast::With;
use crate::model::error::ModelCode;

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
        T: FromStr<Err = crate::model::ast::KeywordError>,
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

/// Reads the FHIRconnect half of the `spec` block.
fn lower_spec(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<ModelSpec> {
    let spec_path = path.field("spec");
    let Some(spec) = node.get("spec") else {
        lowering.missing_key(node, path, "spec");
        return None;
    };
    lowering.mapping(spec, &spec_path)?;
    lowering.refuse_unknown_keys(spec, &spec_path, SPEC_KEYS);

    if let Some(config) = spec.get("openEhrConfig") {
        let config_path = spec_path.field("openEhrConfig");
        if lowering.mapping(config, &config_path).is_some() {
            lowering.refuse_unknown_keys(config, &config_path, OPENEHR_CONFIG_KEYS);
        }
    }

    let structure_definition = match spec.get("fhirConfig") {
        Some(config) => {
            let config_path = spec_path.field("fhirConfig");
            match lowering.mapping(config, &config_path) {
                Some(config) => {
                    lowering.refuse_unknown_keys(config, &config_path, FHIR_CONFIG_KEYS);
                    lowering.required_text(config, &config_path, "structureDefinition")
                }
                None => None,
            }
        }
        None => None,
    };

    let system = lowering.required_text(spec, &spec_path, "system")?;
    let version = lowering.required_text(spec, &spec_path, "version")?;
    let extends = lowering.optional_mapping_name(spec, &spec_path, "extends");
    let conceptmap = lowering.optional_text(spec, &spec_path, "conceptmap");
    let unidirectional = lowering.optional_keyword::<Direction>(
        spec,
        &spec_path,
        "unidirectional",
        ModelCode::InvalidDirection,
    );

    Some(ModelSpec {
        position: spec.position(),
        system,
        version,
        extends,
        structure_definition,
        conceptmap,
        unidirectional,
    })
}

/// Reads the `preprocessor` block, treating a null as absent.
fn lower_preprocessor(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Preprocessor> {
    let block_path = path.field("preprocessor");
    let block = node.get("preprocessor")?;
    if matches!(*block.value(), MappingValue::Null) {
        return None;
    }
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, PREPROCESSOR_KEYS);
    Some(Preprocessor {
        position: block.position(),
        fhir_condition: lower_condition(lowering, block, &block_path, "fhirCondition"),
        openehr_condition: lower_condition(lowering, block, &block_path, "openehrCondition"),
        hierarchy: lower_hierarchy(lowering, block, &block_path),
    })
}

/// Reads the `hierarchy` block of a preprocessor.
fn lower_hierarchy(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Hierarchy> {
    let block_path = path.field("hierarchy");
    let block = node.get("hierarchy")?;
    if matches!(*block.value(), MappingValue::Null) {
        return None;
    }
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, HIERARCHY_KEYS);
    let split = lower_split(lowering, block, &block_path);
    Some(Hierarchy {
        position: block.position(),
        with: lower_with(lowering, block, &block_path),
        split,
    })
}

/// Reads a `hierarchy.split` block.
fn lower_split(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<HierarchySplit> {
    let block_path = path.field("split");
    let block = node.get("split")?;
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, SPLIT_KEYS);
    Some(HierarchySplit {
        position: block.position(),
        fhir: lower_split_target(lowering, block, &block_path, "fhir"),
        openehr: lower_split_target(lowering, block, &block_path, "openehr"),
    })
}

/// Reads one side of a `hierarchy.split`.
fn lower_split_target(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
    key: &str,
) -> Option<SplitTarget> {
    let block_path = path.field(key);
    let block = node.get(key)?;
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, SPLIT_TARGET_KEYS);
    let unique = lowering.text_list(block, &block_path, "unique");
    for (index, entry) in unique.iter().enumerate() {
        lowering.check_path_variable(entry, &block_path.field("unique").index(index));
    }
    Some(SplitTarget {
        position: block.position(),
        create: lowering.optional_text(block, &block_path, "create"),
        path: lowering.optional_path(block, &block_path, "path"),
        unique,
    })
}

/// Reads the `mappings` sequence, refusing a `mappings` written with no value.
fn lower_mappings(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Vec<Mapping> {
    let mappings_path = path.field("mappings");
    let Some(value) = node.get("mappings") else {
        return Vec::new();
    };
    if matches!(*value.value(), MappingValue::Null) {
        lowering.report(
            ModelCode::NullMappings,
            value.position(),
            &mappings_path,
            "`mappings` is written with no value; the published \
             model-mapping.schema.json types it as an array",
        );
        return Vec::new();
    }
    lower_mapping_sequence(lowering, value, &mappings_path)
}

/// Reads a sequence of mapping methods.
fn lower_mapping_sequence(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Vec<Mapping> {
    let Some(items) = node.as_sequence() else {
        lowering.wrong_kind(node, path, ValueKind::Sequence);
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| lower_mapping(lowering, item, &path.index(index)))
        .collect()
}

/// Reads one mapping method.
fn lower_mapping(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Mapping> {
    lowering.mapping(node, path)?;
    lowering.refuse_unknown_keys(node, path, MAPPING_KEYS);
    let name = lowering.required_text(node, path, "name")?;
    let with = lower_with(lowering, node, path);
    // NOTE: the published model schema puts the data-type enum at mapping
    // level and every prose example writes it inside `with`, so both places
    // lower into the one field.
    let mapping_type =
        lowering.optional_keyword::<DataType>(node, path, "type", ModelCode::InvalidDataType);
    let with = match (with, mapping_type) {
        (Some(mut with), Some(data_type)) if with.data_type.is_none() => {
            with.data_type = Some(data_type);
            Some(with)
        }
        (with, _) => with,
    };
    Some(Mapping {
        position: node.position(),
        name,
        extension: lowering.optional_keyword::<ExtensionMethod>(
            node,
            path,
            "extension",
            ModelCode::InvalidExtensionMethod,
        ),
        append_to: lowering.optional_text(node, path, "appendTo"),
        with,
        unidirectional: lowering.optional_keyword::<Direction>(
            node,
            path,
            "unidirectional",
            ModelCode::InvalidDirection,
        ),
        manual: lower_manual(lowering, node, path),
        fhir_condition: lower_condition(lowering, node, path, "fhirCondition"),
        openehr_condition: lower_condition(lowering, node, path, "openehrCondition"),
        followed_by: lower_followed_by(lowering, node, path),
        reference: lower_reference(lowering, node, path),
        slot_archetype: lowering.optional_mapping_name(node, path, "slotArchetype"),
        mapping_code: lowering.optional_text(node, path, "mappingCode"),
        link: lower_link(lowering, node, path),
        participations_function: lowering.optional_text(node, path, "participationsFunction"),
        conceptmap: lowering.optional_text(node, path, "conceptmap"),
    })
}

/// Reads a `with` block.
fn lower_with(lowering: &mut Lowering, node: &PositionedValue, path: &ModelPath) -> Option<With> {
    let block_path = path.field("with");
    let block = node.get("with")?;
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, WITH_KEYS);
    Some(With {
        position: block.position(),
        fhir: lowering.optional_path(block, &block_path, "fhir"),
        openehr: lowering.optional_path(block, &block_path, "openehr"),
        data_type: lowering.optional_keyword::<DataType>(
            block,
            &block_path,
            "type",
            ModelCode::InvalidDataType,
        ),
        value: lowering.optional_text(block, &block_path, "value"),
    })
}

/// Reads a `followedBy` block, treating a null as absent.
fn lower_followed_by(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<FollowedBy> {
    let block_path = path.field("followedBy");
    let block = node.get("followedBy")?;
    if matches!(*block.value(), MappingValue::Null) {
        return None;
    }
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, &["mappings"]);
    let mappings_path = block_path.field("mappings");
    let Some(mappings) = block.get("mappings") else {
        lowering.missing_key(block, &block_path, "mappings");
        return None;
    };
    Some(FollowedBy {
        position: block.position(),
        mappings: lower_mapping_sequence(lowering, mappings, &mappings_path),
    })
}

/// Reads a `reference` block, treating a null as absent.
fn lower_reference(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<ReferenceMapping> {
    let block_path = path.field("reference");
    let block = node.get("reference")?;
    if matches!(*block.value(), MappingValue::Null) {
        return None;
    }
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, &["resourceType", "mappings"]);
    let resource_type = lowering.required_text(block, &block_path, "resourceType")?;
    let mappings_path = block_path.field("mappings");
    let Some(mappings) = block.get("mappings") else {
        lowering.missing_key(block, &block_path, "mappings");
        return None;
    };
    Some(ReferenceMapping {
        position: block.position(),
        resource_type,
        mappings: lower_mapping_sequence(lowering, mappings, &mappings_path),
    })
}

/// Reads a `link` block.
fn lower_link(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<LinkMapping> {
    let block_path = path.field("link");
    let block = node.get("link")?;
    if matches!(*block.value(), MappingValue::Null) {
        return None;
    }
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, LINK_KEYS);
    Some(LinkMapping {
        position: block.position(),
        meaning: lowering.optional_text(block, &block_path, "meaning"),
        link_type: lowering.optional_text(block, &block_path, "type"),
    })
}

/// Reads a `manual` sequence.
fn lower_manual(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Vec<ManualEntry> {
    let block_path = path.field("manual");
    let Some(block) = node.get("manual") else {
        return Vec::new();
    };
    let Some(items) = block.as_sequence() else {
        lowering.wrong_kind(block, &block_path, ValueKind::Sequence);
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| lower_manual_entry(lowering, item, &block_path.index(index)))
        .collect()
}

/// Reads one `manual` entry.
fn lower_manual_entry(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<ManualEntry> {
    lowering.mapping(node, path)?;
    lowering.refuse_unknown_keys(node, path, MANUAL_KEYS);
    let name = lowering.required_text(node, path, "name")?;
    Some(ManualEntry {
        position: node.position(),
        name,
        fhir: lower_manual_paths(lowering, node, path, "fhir"),
        openehr: lower_manual_paths(lowering, node, path, "openehr"),
        fhir_condition: lower_condition(lowering, node, path, "fhirCondition"),
        openehr_condition: lower_condition(lowering, node, path, "openehrCondition"),
        value: lowering.optional_text(node, path, "value"),
        unidirectional: lowering.optional_keyword::<Direction>(
            node,
            path,
            "unidirectional",
            ModelCode::InvalidDirection,
        ),
    })
}

/// Reads the `path` and `value` pairs of one side of a manual entry.
///
/// The specification writes both a bare pair and a list of pairs: the manual
/// chapter writes a list
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/manual.adoc`)
/// and the `$openehrRoot` example writes one pair
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`), so a
/// lone pair lowers into a one-element list.
fn lower_manual_paths(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
    key: &str,
) -> Vec<ManualPath> {
    let block_path = path.field(key);
    let Some(block) = node.get(key) else {
        return Vec::new();
    };
    match block.as_sequence() {
        Some(items) => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| lower_manual_path(lowering, item, &block_path.index(index)))
            .collect(),
        None => lower_manual_path(lowering, block, &block_path)
            .into_iter()
            .collect(),
    }
}

/// Reads one `path` and `value` pair.
fn lower_manual_path(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<ManualPath> {
    lowering.mapping(node, path)?;
    lowering.refuse_unknown_keys(node, path, MANUAL_PATH_KEYS);
    let manual_path = lowering.required_text(node, path, "path")?;
    lowering.check_path_variable(&manual_path, &path.field("path"));
    let value = lowering.required_text(node, path, "value")?;
    Some(ManualPath {
        position: node.position(),
        path: manual_path,
        value,
    })
}

/// Reads one condition under `key`, treating a null as absent.
fn lower_condition(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
    key: &str,
) -> Option<Condition> {
    let block_path = path.field(key);
    let block = node.get(key)?;
    if matches!(*block.value(), MappingValue::Null) {
        return None;
    }
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, CONDITION_KEYS);

    let target_root = lowering.required_text(block, &block_path, "targetRoot")?;
    lowering.check_path_variable(&target_root, &block_path.field("targetRoot"));

    let mut target_attributes = lowering.text_list(block, &block_path, "targetAttributes");
    target_attributes.extend(lowering.text_list(block, &block_path, "targetAttribute"));
    let mut criteria = lowering.text_list(block, &block_path, "criterias");
    criteria.extend(lowering.text_list(block, &block_path, "criteria"));

    let operator_path = block_path.field("operator");
    let operator_text = lowering.required_text(block, &block_path, "operator")?;
    let operator = match ConditionOperator::from_str(operator_text.value()) {
        Ok(operator) => Located::new(operator_text.position(), operator),
        Err(error) => {
            lowering.report(
                ModelCode::InvalidOperator,
                operator_text.position(),
                &operator_path,
                error.to_string(),
            );
            return None;
        }
    };

    let identifying = block
        .get("identifying")
        .and_then(|value| lowering.boolean(value, &block_path.field("identifying")));

    Some(Condition {
        position: block.position(),
        target_root,
        target_attributes,
        operator,
        criteria,
        identifying,
    })
}

/// Reads the `context` block of a context mapping file.
fn lower_context_block(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<MappingContext> {
    let block_path = path.field("context");
    let Some(block) = node.get("context") else {
        lowering.missing_key(node, path, "context");
        return None;
    };
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, CONTEXT_KEYS);

    let profile = lower_profile(lowering, block, &block_path)?;
    let template = lower_template(lowering, block, &block_path)?;
    let archetypes = lower_name_list(lowering, block, &block_path, "archetypes", true);
    let extensions = lower_name_list(lowering, block, &block_path, "extensions", false);
    let operational = lower_name_list(lowering, block, &block_path, "operational", false);
    let start_text = lowering.required_text(block, &block_path, "start")?;
    let start = lowering.mapping_name(&start_text, &block_path.field("start"))?;

    Some(MappingContext {
        position: block.position(),
        profile,
        template,
        archetypes,
        extensions,
        operational,
        start,
    })
}

/// Reads `context.profile`.
fn lower_profile(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<ProfileRef> {
    let block_path = path.field("profile");
    let Some(block) = node.get("profile") else {
        lowering.missing_key(node, path, "profile");
        return None;
    };
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, PROFILE_KEYS);
    Some(ProfileRef {
        position: block.position(),
        url: lowering.optional_text(block, &block_path, "url"),
        version: lowering.optional_text(block, &block_path, "version"),
    })
}

/// Reads `context.template`.
fn lower_template(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<TemplateRef> {
    let block_path = path.field("template");
    let Some(block) = node.get("template") else {
        lowering.missing_key(node, path, "template");
        return None;
    };
    lowering.mapping(block, &block_path)?;
    lowering.refuse_unknown_keys(block, &block_path, TEMPLATE_KEYS);
    Some(TemplateRef {
        position: block.position(),
        id: lowering.optional_text(block, &block_path, "id"),
        sem_ver: lowering.optional_text(block, &block_path, "sem_ver"),
    })
}

/// Reads one of the `context` lists of mapping names.
fn lower_name_list(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
    key: &str,
    required: bool,
) -> Vec<Located<MappingName>> {
    let list_path = path.field(key);
    let Some(value) = node.get(key) else {
        if required {
            lowering.missing_key(node, path, key);
        }
        return Vec::new();
    };
    let Some(items) = value.as_sequence() else {
        lowering.wrong_kind(value, &list_path, ValueKind::Sequence);
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let entry_path = list_path.index(index);
            let text = lowering.text(item, &entry_path)?;
            lowering.mapping_name(&text, &entry_path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::loader::load_str;
    use openehr_mapping_core::position::Position;

    use super::lower_context;
    use super::lower_model;
    use crate::model::ast::ConditionOperator;
    use crate::model::ast::DataType;
    use crate::model::ast::Direction;
    use crate::model::error::ModelCode;

    /// The smallest model mapping file the grammar admits, plus `mappings`.
    fn model(body: &str) -> String {
        format!(
            "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: EVALUATION.test.v1\n  \
             version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    \
             archetype: openEHR-EHR-EVALUATION.test.v1\n{body}"
        )
    }

    #[test]
    fn a_minimal_model_file_lowers() {
        let document = load_str(
            "model.yml",
            &model(
                "mappings:\n  - name: \"dateTime\"\n    with:\n      fhir: \"$resource.onset\"\n \
                 \x20    openehr: \"$archetype/data[at0001]/items[at0077]\"\n",
            ),
        )
        .expect("a well-formed header");
        let file = lower_model(&document).expect("a well-formed model file");
        assert_eq!(file.mappings().len(), 1);
        let mapping = file.mappings().first().expect("one mapping");
        assert_eq!(mapping.name.value(), "dateTime");
        let with = mapping.with.as_ref().expect("a with block");
        assert_eq!(
            with.openehr.as_ref().map(|p| p.value().as_str()),
            Some("$archetype/data[at0001]/items[at0077]")
        );
    }

    #[test]
    fn the_singular_and_plural_condition_spellings_lower_alike() {
        let singular = load_str(
            "singular.yml",
            &model(
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
                 openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
                 \"$resource.code\"\n      targetAttribute: \"coding.code\"\n      operator: \"one \
                 of\"\n      criteria: \"x\"\n",
            ),
        )
        .expect("a well-formed header");
        let plural = load_str(
            "plural.yml",
            &model(
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
                 openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
                 \"$resource.code\"\n      targetAttributes:\n        - \"coding.code\"\n      \
                 operator: \"one of\"\n      criterias:\n        - \"x\"\n",
            ),
        )
        .expect("a well-formed header");
        let singular = lower_model(&singular).expect("the singular spelling lowers");
        let plural = lower_model(&plural).expect("the plural spelling lowers");
        let left = singular
            .mappings()
            .first()
            .and_then(|m| m.fhir_condition.as_ref())
            .expect("a condition");
        let right = plural
            .mappings()
            .first()
            .and_then(|m| m.fhir_condition.as_ref())
            .expect("a condition");
        let attributes = |c: &crate::model::ast::Condition| {
            c.target_attributes
                .iter()
                .map(|a| a.value().clone())
                .collect::<Vec<String>>()
        };
        let criteria = |c: &crate::model::ast::Condition| {
            c.criteria
                .iter()
                .map(|a| a.value().clone())
                .collect::<Vec<String>>()
        };
        assert_eq!(attributes(left), attributes(right));
        assert_eq!(criteria(left), criteria(right));
        assert_eq!(*left.operator.value(), ConditionOperator::OneOf);
    }

    #[test]
    fn a_two_attribute_condition_keeps_both_attributes() {
        let document = load_str(
            "two.yml",
            &model(
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
                 openehr: \"$archetype\"\n    openehrCondition:\n      targetRoot: \
                 \"$archetype\"\n      targetAttributes:\n        - \"items[at0071]\"\n        - \
                 \"items[at0115]\"\n      operator: \"empty\"\n",
            ),
        )
        .expect("a well-formed header");
        let file = lower_model(&document).expect("a well-formed model file");
        let condition = file
            .mappings()
            .first()
            .and_then(|m| m.openehr_condition.as_ref())
            .expect("a condition");
        let attributes: Vec<&str> = condition
            .target_attributes
            .iter()
            .map(|a| a.value().as_str())
            .collect();
        assert_eq!(attributes, vec!["items[at0071]", "items[at0115]"]);
    }

    #[test]
    fn a_null_mappings_is_refused() {
        let document = load_str("null.yml", &model("mappings:\n")).expect("a well-formed header");
        let diagnostics = lower_model(&document).expect_err("a null mappings");
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::NullMappings.into());
    }

    #[test]
    fn a_root_unidirectional_is_refused_naming_the_schema() {
        let document = load_str(
            "root.yml",
            &model("unidirectional: \"openehr->fhir\"\nmappings: []\n"),
        )
        .expect("a well-formed header");
        let diagnostics = lower_model(&document).expect_err("a root unidirectional");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::RootUnidirectional.into());
        assert!(
            diagnostic.message().contains("model-mapping.schema.json"),
            "{}",
            diagnostic.message()
        );
    }

    #[test]
    fn a_spec_unidirectional_is_accepted() {
        let document = load_str(
            "spec.yml",
            "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: provenance\n  version: \
             1.0.0\nspec:\n  system: FHIR\n  version: R4\n  unidirectional: \
             \"openehr->fhir\"\nmappings: []\n",
        )
        .expect("a well-formed header");
        let file = lower_model(&document).expect("a well-formed operational model file");
        assert_eq!(
            file.spec().unidirectional.as_ref().map(|d| *d.value()),
            Some(Direction::OpenehrToFhir)
        );
    }

    #[test]
    fn an_unknown_path_variable_is_refused() {
        let document = load_str(
            "variable.yml",
            &model(
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
                 openehr: \"$archetypeRoot/data\"\n",
            ),
        )
        .expect("a well-formed header");
        let diagnostics = lower_model(&document).expect_err("an unknown variable");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::UnknownPathVariable.into());
    }

    #[test]
    fn a_mis_cased_variable_is_accepted() {
        let document = load_str(
            "cased.yml",
            &model(
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$fhirRoot\"\n      openehr: \
                 \"$openEHRRoot/defining_code\"\n",
            ),
        )
        .expect("a well-formed header");
        let file = lower_model(&document).expect("a mis-cased variable is admitted");
        assert_eq!(file.mappings().len(), 1);
    }

    #[test]
    fn an_unknown_mapping_key_is_refused() {
        let document = load_str(
            "unknown.yml",
            &model("mappings:\n  - name: \"a\"\n    hardcodedValue: \"x\"\n"),
        )
        .expect("a well-formed header");
        let diagnostics = lower_model(&document).expect_err("an unknown key");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::UnknownKey.into());
        assert_eq!(
            diagnostic.model_path().to_string(),
            "mappings[0].hardcodedValue"
        );
    }

    #[test]
    fn a_data_type_inside_with_lowers() {
        let document = load_str(
            "type.yml",
            &model(
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource\"\n      openehr: \
                 \"$archetype\"\n      type: \"NONE\"\n",
            ),
        )
        .expect("a well-formed header");
        let file = lower_model(&document).expect("a well-formed model file");
        let with = file
            .mappings()
            .first()
            .and_then(|m| m.with.as_ref())
            .expect("a with block");
        assert_eq!(
            with.data_type.as_ref().map(|t| *t.value()),
            Some(DataType::None)
        );
    }

    #[test]
    fn a_context_file_lowers() {
        let document = load_str(
            "context.yml",
            "grammar: FHIRConnect/v1.0.0\ntype: context\nmetadata:\n  name: KDS.context\n  \
             version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\ncontext:\n  profile:\n    url: \
             \"https://example.invalid/StructureDefinition/Procedure\"\n    version: \
             \"2025.0.0\"\n  template:\n    id: \"KDS_Prozedur\"\n    sem_ver: \"10.0.0\"\n  \
             archetypes:\n    - \"ACTION.procedure.v1\"\n  extensions:\n    - \
             \"KDS_procedure.v1\"\n  start: \"ACTION.procedure.v1\"\n",
        )
        .expect("a well-formed header");
        let file = lower_context(&document).expect("a well-formed context file");
        assert_eq!(file.context().archetypes.len(), 1);
        assert_eq!(file.context().start.value().as_str(), "ACTION.procedure.v1");
        assert_eq!(file.context().position, Position::new(10, 3));
    }

    #[test]
    fn a_model_file_is_refused_by_the_context_entry_point() {
        let document = load_str("model.yml", &model("mappings: []\n")).expect("a header");
        let diagnostics = lower_context(&document).expect_err("the wrong file type");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::WrongMappingFileType.into());
    }
}
