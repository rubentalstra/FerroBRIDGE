// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowering from a positioned YAML tree to the OMOCL file model.
//!
//! The loader in `openehr-mapping-core` parses the YAML, resolves anchors and
//! aliases and reads the shared header; this module reads the OMOCL grammar
//! out of the tree it returns. It refuses a key the grammar does not define,
//! a node of the wrong kind and a value outside its admitted set, positioned
//! at the key or value at fault, and it collects every refusal of a file
//! rather than stopping at the first.

use core::str::FromStr;
use std::collections::BTreeMap;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::ArchetypeId;
use openehr_mapping_core::header::GrammarSemVer;
use openehr_mapping_core::header::MappingLanguage;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::header::MappingType;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::value::MappingEntry;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::Alternative;
use crate::model::ast::AtCode;
use crate::model::ast::CdmSystem;
use crate::model::ast::CdmVersion;
use crate::model::ast::Column;
use crate::model::ast::ConceptId;
use crate::model::ast::ConceptMap;
use crate::model::ast::CustomMapping;
use crate::model::ast::Entity;
use crate::model::ast::EntityType;
use crate::model::ast::Factor;
use crate::model::ast::Include;
use crate::model::ast::MappingFile;
use crate::model::ast::Multiplication;
use crate::model::ast::Record;
use crate::model::ast::Spec;
use crate::model::ast::Target;
use crate::model::error::ModelCode;
use crate::model::projection::key_without_column;
use crate::model::projection::projection;

/// The grammar version this crate implements, the version of [`crate::GRAMMAR`].
pub const GRAMMAR_VERSION: GrammarSemVer = GrammarSemVer::new(1, 0, 0);

/// The keys a mapping file writes at the document root.
pub const ROOT_KEYS: &[&str] = &["grammar", "type", "metadata", "spec", "mappings"];

/// The keys `metadata` writes.
pub const METADATA_KEYS: &[&str] = &["name", "version"];

/// The keys `spec` writes.
pub const SPEC_KEYS: &[&str] = &["system", "version", "openEhrConfig"];

/// The keys `spec.openEhrConfig` writes.
pub const OPENEHR_CONFIG_KEYS: &[&str] = &["archetype"];

/// The keys a column entry writes, the railroad's CONVERSION clause.
pub const COLUMN_KEYS: &[&str] = &["optional", "alternatives"];

/// The four forms an alternative writes, the railroad's CDM FIELD clause and
/// the library's `multiplication`.
pub const ALTERNATIVE_KEYS: &[&str] = &["path", "code", "conceptMap", "multiplication"];

/// The keys a `conceptMap` writes.
pub const CONCEPT_MAP_KEYS: &[&str] = &["path", "mapping"];

/// The keys one `multiplication` factor writes, exactly one per factor.
pub const FACTOR_KEYS: &[&str] = &["path", "code"];

/// The keys an `Include` writes beside `type`.
pub const INCLUDE_KEYS: &[&str] = &["type", "archetype_id", "base_path"];

/// The keys a `CustomMapping` writes beside `type`.
pub const CUSTOM_MAPPING_KEYS: &[&str] = &["type", "name"];

/// The collector one file's lowering runs against.
#[derive(Debug)]
struct Lowering {
    file: PathBuf,
    mapping_name: MappingName,
    diagnostics: Vec<Diagnostic>,
}

impl Lowering {
    /// Records one refusal.
    fn report(
        &mut self,
        code: ModelCode,
        position: Position,
        path: &ModelPath,
        message: impl Into<String>,
    ) {
        self.push(
            Diagnostic::error(self.file.clone(), code.into(), message),
            position,
            path,
        );
    }

    /// Records one refusal the shared foundation already rendered.
    fn push(&mut self, diagnostic: Diagnostic, position: Position, path: &ModelPath) {
        self.diagnostics.push(
            diagnostic
                .with_position(position)
                .with_model_path(path.clone())
                .with_mapping_name(self.mapping_name.clone()),
        );
    }

    /// Refuses every key of `node` outside `admitted`, naming each one.
    fn refuse_unknown_keys(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
        admitted: &[&str],
        what: &str,
    ) {
        let Some(entries) = node.as_mapping() else {
            return;
        };
        for (key, entry) in entries {
            if !admitted.contains(&key.as_str()) {
                self.report(
                    ModelCode::UnknownKey,
                    entry.key_position(),
                    &path.field(key),
                    format!(
                        "{what} has no key `{key}`; it admits {}",
                        render_admitted(admitted)
                    ),
                );
            }
        }
    }

    /// Returns `node` as a mapping, or records that it is not one.
    fn mapping<'a>(
        &mut self,
        node: &'a PositionedValue,
        path: &ModelPath,
    ) -> Option<&'a BTreeMap<String, MappingEntry>> {
        let entries = node.as_mapping();
        if entries.is_none() {
            self.wrong_kind(node, path, ValueKind::Mapping);
        }
        entries
    }

    /// Returns `node` as a sequence, or records that it is not one.
    fn sequence<'a>(
        &mut self,
        node: &'a PositionedValue,
        path: &ModelPath,
    ) -> Option<&'a [PositionedValue]> {
        let items = node.as_sequence();
        if items.is_none() {
            self.wrong_kind(node, path, ValueKind::Sequence);
        }
        items
    }

    /// Returns `node` as a string, or records that it is not one.
    fn text<'a>(&mut self, node: &'a PositionedValue, path: &ModelPath) -> Option<&'a str> {
        let text = node.as_text();
        if text.is_none() {
            self.wrong_kind(node, path, ValueKind::Text);
        }
        text
    }

    /// Records that `node` is not of the `expected` kind.
    fn wrong_kind(&mut self, node: &PositionedValue, path: &ModelPath, expected: ValueKind) {
        self.report(
            ModelCode::UnexpectedNodeKind,
            node.position(),
            path,
            format!("expected {expected}, found {}", node.kind()),
        );
    }

    /// Returns the entry `key` of `node`, or records that it is absent.
    fn required<'a>(
        &mut self,
        node: &'a PositionedValue,
        path: &ModelPath,
        key: &str,
    ) -> Option<&'a MappingEntry> {
        let entry = node.entry(key);
        if entry.is_none() {
            self.report(
                ModelCode::MissingKey,
                node.position(),
                path,
                format!("the key `{key}` is required here"),
            );
        }
        entry
    }

    /// Parses a mapping path, or records why it was refused.
    fn path(&mut self, node: &PositionedValue, path: &ModelPath) -> Option<Located<MappingPath>> {
        let text = self.text(node, path)?;
        match MappingPath::from_str(text) {
            Ok(parsed) => Some(Located::new(node.position(), parsed)),
            Err(error) => {
                let diagnostic = error.to_diagnostic(self.file.clone());
                self.push(diagnostic, node.position(), path);
                None
            }
        }
    }

    /// Reads a literal concept id, or records why it was refused.
    fn concept_id(
        &mut self,
        node: &PositionedValue,
        path: &ModelPath,
    ) -> Option<Located<ConceptId>> {
        let number = match *node.value() {
            MappingValue::Signed(value) => i32::try_from(value).ok(),
            MappingValue::Unsigned(value) => i32::try_from(value).ok(),
            _ => {
                self.wrong_kind(node, path, ValueKind::Number);
                return None;
            }
        };
        // NOTE: an out-of-range number is refused below with its own code, so
        // the `Option` here is the answer to "does it fit a CDM integer".
        let Some(number) = number else {
            self.report(
                ModelCode::InvalidConceptId,
                node.position(),
                path,
                "a concept id is a CDM `integer`, a 32-bit signed number",
            );
            return None;
        };
        Some(Located::new(node.position(), ConceptId::new(number)))
    }
}

/// Renders an admitted set as a comma-separated backticked list.
fn render_admitted(admitted: &[&str]) -> String {
    admitted
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<String>>()
        .join(", ")
}

/// Lowers one loaded document into the OMOCL file model.
///
/// # Errors
///
/// Returns every refusal the lowering raised, each positioned at the YAML node
/// it is about and carrying the file's mapping name.
pub fn lower(document: &MappingDocument) -> Result<MappingFile, Vec<Diagnostic>> {
    let header = document.header();
    let mut lowering = Lowering {
        file: document.file().to_path_buf(),
        mapping_name: header.name().value().clone(),
        diagnostics: Vec::new(),
    };
    let root = document.document();
    let root_path = ModelPath::root();
    lowering.refuse_unknown_keys(root, &root_path, ROOT_KEYS, "a mapping file");

    let grammar = header.grammar();
    if grammar.value().language() != MappingLanguage::Omocl
        || grammar.value().spelling() != MappingLanguage::Omocl.canonical_spelling()
        || grammar.value().version() != GRAMMAR_VERSION
    {
        lowering.report(
            ModelCode::UnsupportedGrammar,
            grammar.position(),
            &root_path.field("grammar"),
            format!(
                "the grammar is `{}`; this crate implements `{}`",
                grammar.value(),
                crate::GRAMMAR
            ),
        );
    }
    if let Some(mapping_type) = header.mapping_type()
        && *mapping_type.value() != MappingType::Model
    {
        lowering.report(
            ModelCode::UnsupportedFileType,
            mapping_type.position(),
            &root_path.field("type"),
            format!(
                "an OMOCL file is `type: model`, found `{}`",
                mapping_type.value()
            ),
        );
    }
    if let Some(metadata) = root.get("metadata") {
        lowering.refuse_unknown_keys(
            metadata,
            &root_path.field("metadata"),
            METADATA_KEYS,
            "`metadata`",
        );
    }

    let spec = lower_spec(&mut lowering, root, &root_path);
    let archetype = header.archetype().cloned();
    if archetype.is_none() {
        lowering.report(
            ModelCode::MissingKey,
            header.spec().position(),
            &root_path.field("spec").field("openEhrConfig"),
            "`spec.openEhrConfig.archetype` names the mapped archetype and is required",
        );
    }
    let entities = lower_mappings(&mut lowering, root, &root_path);

    match (spec, archetype, entities) {
        (Some(spec), Some(archetype), Some(entities)) if lowering.diagnostics.is_empty() => {
            Ok(MappingFile {
                file: lowering.file,
                header: header.clone(),
                archetype,
                spec,
                entities,
            })
        }
        _ => Err(lowering.diagnostics),
    }
}

/// Lowers `spec` below the shared header.
fn lower_spec(
    lowering: &mut Lowering,
    root: &PositionedValue,
    root_path: &ModelPath,
) -> Option<Spec> {
    let path = root_path.field("spec");
    let spec = lowering.required(root, root_path, "spec")?.value();
    lowering.mapping(spec, &path)?;
    lowering.refuse_unknown_keys(spec, &path, SPEC_KEYS, "`spec`");

    let system_path = path.field("system");
    let system = lowering.required(spec, &path, "system").and_then(|entry| {
        let node = entry.value();
        let text = lowering.text(node, &system_path)?;
        if text == "OMOP" {
            Some(Located::new(node.position(), CdmSystem::Omop))
        } else {
            lowering.report(
                ModelCode::UnsupportedTarget,
                node.position(),
                &system_path,
                format!("`spec.system` is `OMOP`, found `{text}`"),
            );
            None
        }
    });

    let version_path = path.field("version");
    let version = lowering.required(spec, &path, "version").and_then(|entry| {
        let node = entry.value();
        // NOTE: no specification governs this: our own design; the library
        // writes the YAML float `5.4` and a quoted `"5.4"` names the same release.
        let written = match *node.value() {
            MappingValue::Float(value) => Some(value.to_string()),
            MappingValue::Text(ref value) => Some(value.clone()),
            _ => None,
        };
        if written.as_deref() == Some("5.4") {
            Some(Located::new(node.position(), CdmVersion::V5_4))
        } else {
            lowering.report(
                ModelCode::UnsupportedTarget,
                node.position(),
                &version_path,
                "`spec.version` is the OMOP CDM release `5.4`",
            );
            None
        }
    });

    let config_path = path.field("openEhrConfig");
    if let Some(entry) = lowering.required(spec, &path, "openEhrConfig") {
        lowering.refuse_unknown_keys(
            entry.value(),
            &config_path,
            OPENEHR_CONFIG_KEYS,
            "`spec.openEhrConfig`",
        );
    }

    Some(Spec {
        position: spec.position(),
        system: system?,
        version: version?,
    })
}

/// Lowers `mappings`, the railroad's list of CDT clauses.
fn lower_mappings(
    lowering: &mut Lowering,
    root: &PositionedValue,
    root_path: &ModelPath,
) -> Option<Vec<Entity>> {
    let path = root_path.field("mappings");
    let node = lowering.required(root, root_path, "mappings")?.value();
    let items = lowering.sequence(node, &path)?;
    if items.is_empty() {
        lowering.report(
            ModelCode::EmptyList,
            node.position(),
            &path,
            "`mappings` holds at least one entry",
        );
        return None;
    }
    let mut entities = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if let Some(entity) = lower_entity(lowering, item, &path.index(index)) {
            entities.push(entity);
        }
    }
    (entities.len() == items.len()).then_some(entities)
}

/// Lowers one entry of `mappings`.
fn lower_entity(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Entity> {
    lowering.mapping(node, path)?;
    let type_path = path.field("type");
    let type_node = lowering.required(node, path, "type")?.value();
    let text = lowering.text(type_node, &type_path)?;
    let entity_type = match EntityType::from_str(text) {
        Ok(entity_type) => entity_type,
        Err(error) => {
            lowering.report(
                ModelCode::UnknownType,
                type_node.position(),
                &type_path,
                format!(
                    "{error}; `type` admits {}",
                    render_admitted(EntityType::ADMITTED)
                ),
            );
            return None;
        }
    };
    match entity_type {
        EntityType::Target(target) => lower_record(
            lowering,
            node,
            path,
            Located::new(type_node.position(), target),
        )
        .map(Entity::Record),
        EntityType::Include => lower_include(lowering, node, path).map(Entity::Include),
        EntityType::CustomMapping => {
            lower_custom_mapping(lowering, node, path).map(Entity::CustomMapping)
        }
    }
}

/// Lowers a record under a CDM target.
fn lower_record(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
    target: Located<Target>,
) -> Option<Record> {
    let entries = node.as_mapping()?;
    let admitted = projection(*target.value());
    let mut ok = true;
    let mut base_path = None;
    let mut columns = Vec::new();
    for (key, entry) in entries {
        let key_path = path.field(key);
        match key.as_str() {
            "type" => {}
            "base_path" => match lowering.path(entry.value(), &key_path) {
                Some(parsed) => base_path = Some(parsed),
                None => ok = false,
            },
            _ => {
                if let Some(projection) = admitted.key(key) {
                    match lower_column(lowering, entry, &key_path, projection) {
                        Some(column) => columns.push(column),
                        None => ok = false,
                    }
                } else if let Some(without) = key_without_column(*target.value(), key) {
                    ok = false;
                    lowering.report(
                        ModelCode::KeyWithoutColumn,
                        entry.key_position(),
                        &key_path,
                        format!(
                            "`{key}` under `{}` writes no CDM column: {}",
                            target.value(),
                            without.reason
                        ),
                    );
                } else {
                    ok = false;
                    let keys: Vec<&str> = admitted.keys.iter().map(|k| k.key).collect();
                    lowering.report(
                        ModelCode::UnknownKey,
                        entry.key_position(),
                        &key_path,
                        format!(
                            "`{}` has no key `{key}`; it admits `base_path` and {}",
                            target.value(),
                            render_admitted(&keys)
                        ),
                    );
                }
            }
        }
    }
    ok.then_some(Record {
        position: node.position(),
        target,
        base_path,
        columns,
    })
}

/// Lowers one column entry, the railroad's CONVERSION clause.
fn lower_column(
    lowering: &mut Lowering,
    entry: &MappingEntry,
    path: &ModelPath,
    projection: &'static crate::model::projection::KeyProjection,
) -> Option<Column> {
    let node = entry.value();
    lowering.mapping(node, path)?;
    lowering.refuse_unknown_keys(node, path, COLUMN_KEYS, "a column entry");
    let mut ok = node.as_mapping().is_some_and(|entries| {
        entries
            .keys()
            .all(|key| COLUMN_KEYS.contains(&key.as_str()))
    });

    let optional = node.get("optional").and_then(|value| {
        if let MappingValue::Bool(flag) = *value.value() {
            Some(Located::new(value.position(), flag))
        } else {
            lowering.wrong_kind(value, &path.field("optional"), ValueKind::Bool);
            ok = false;
            None
        }
    });

    let alternatives_path = path.field("alternatives");
    let alternatives = lowering
        .required(node, path, "alternatives")
        .and_then(|entry| lower_alternatives(lowering, entry.value(), &alternatives_path));

    match alternatives {
        Some(alternatives) if ok => Some(Column {
            key_position: entry.key_position(),
            projection,
            optional,
            alternatives,
        }),
        _ => None,
    }
}

/// Lowers an `alternatives` list.
fn lower_alternatives(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Vec<Alternative>> {
    let items = lowering.sequence(node, path)?;
    if items.is_empty() {
        lowering.report(
            ModelCode::EmptyList,
            node.position(),
            path,
            "`alternatives` holds at least one alternative",
        );
        return None;
    }
    let mut alternatives = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if let Some(alternative) = lower_alternative(lowering, item, &path.index(index)) {
            alternatives.push(alternative);
        }
    }
    (alternatives.len() == items.len()).then_some(alternatives)
}

/// Lowers one alternative, which writes exactly one of the four forms.
fn lower_alternative(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Alternative> {
    let entries = lowering.mapping(node, path)?;
    lowering.refuse_unknown_keys(node, path, ALTERNATIVE_KEYS, "an alternative");
    let written: Vec<(&String, &MappingEntry)> = entries
        .iter()
        .filter(|(key, _)| ALTERNATIVE_KEYS.contains(&key.as_str()))
        .collect();
    if written.len() != entries.len() {
        return None;
    }
    let [(key, entry)] = written.as_slice() else {
        let (code, message) = if written.is_empty() {
            (
                ModelCode::EmptyAlternative,
                format!(
                    "an alternative writes one of {}",
                    render_admitted(ALTERNATIVE_KEYS)
                ),
            )
        } else {
            let keys: Vec<&str> = written.iter().map(|(key, _)| key.as_str()).collect();
            (
                ModelCode::AmbiguousAlternative,
                format!(
                    "an alternative writes exactly one of {}, found {}",
                    render_admitted(ALTERNATIVE_KEYS),
                    render_admitted(&keys)
                ),
            )
        };
        lowering.report(code, node.position(), path, message);
        return None;
    };
    let value = entry.value();
    let value_path = path.field(key.as_str());
    match key.as_str() {
        "path" => lowering.path(value, &value_path).map(Alternative::Path),
        "code" => lowering
            .concept_id(value, &value_path)
            .map(Alternative::Code),
        "conceptMap" => {
            lower_concept_map(lowering, entry, &value_path).map(Alternative::ConceptMap)
        }
        _ => lower_multiplication(lowering, entry, &value_path).map(Alternative::Multiplication),
    }
}

/// Lowers a `conceptMap` alternative.
fn lower_concept_map(
    lowering: &mut Lowering,
    entry: &MappingEntry,
    path: &ModelPath,
) -> Option<ConceptMap> {
    let node = entry.value();
    lowering.mapping(node, path)?;
    lowering.refuse_unknown_keys(node, path, CONCEPT_MAP_KEYS, "a `conceptMap`");
    let path_value = lowering
        .required(node, path, "path")
        .and_then(|entry| lowering.path(entry.value(), &path.field("path")));

    let mapping_path = path.field("mapping");
    let mapping = lowering.required(node, path, "mapping").and_then(|entry| {
        let table = entry.value();
        let entries = lowering.mapping(table, &mapping_path)?;
        if entries.is_empty() {
            lowering.report(
                ModelCode::EmptyList,
                table.position(),
                &mapping_path,
                "a `conceptMap` `mapping` holds at least one at-code",
            );
            return None;
        }
        let mut mapping = BTreeMap::new();
        let mut ok = true;
        for (key, entry) in entries {
            let key_path = mapping_path.field(key);
            let code = match AtCode::from_str(key) {
                Ok(code) => Some(code),
                Err(error) => {
                    lowering.report(
                        ModelCode::InvalidAtCode,
                        entry.key_position(),
                        &key_path,
                        error.to_string(),
                    );
                    None
                }
            };
            let concept = lowering.concept_id(entry.value(), &key_path);
            match (code, concept) {
                (Some(code), Some(concept)) => {
                    mapping.insert(code, concept);
                }
                _ => ok = false,
            }
        }
        ok.then_some(mapping)
    });

    let known = node.as_mapping().is_some_and(|entries| {
        entries
            .keys()
            .all(|key| CONCEPT_MAP_KEYS.contains(&key.as_str()))
    });
    match (path_value, mapping) {
        (Some(path_value), Some(mapping)) if known => Some(ConceptMap {
            position: entry.key_position(),
            path: path_value,
            mapping,
        }),
        _ => None,
    }
}

/// Lowers a `multiplication` alternative.
fn lower_multiplication(
    lowering: &mut Lowering,
    entry: &MappingEntry,
    path: &ModelPath,
) -> Option<Multiplication> {
    let node = entry.value();
    let items = lowering.sequence(node, path)?;
    if items.is_empty() {
        lowering.report(
            ModelCode::EmptyList,
            node.position(),
            path,
            "a `multiplication` holds at least one factor",
        );
        return None;
    }
    let mut factors = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if let Some(factor) = lower_factor(lowering, item, &path.index(index)) {
            factors.push(factor);
        }
    }
    (factors.len() == items.len()).then_some(Multiplication {
        position: entry.key_position(),
        factors,
    })
}

/// Lowers one `multiplication` factor, which writes exactly one of `path` and
/// `code`.
fn lower_factor(
    lowering: &mut Lowering,
    item: &PositionedValue,
    path: &ModelPath,
) -> Option<Factor> {
    let entries = lowering.mapping(item, path)?;
    lowering.refuse_unknown_keys(item, path, FACTOR_KEYS, "a `multiplication` factor");
    let written: Vec<(&String, &MappingEntry)> = entries
        .iter()
        .filter(|(key, _)| FACTOR_KEYS.contains(&key.as_str()))
        .collect();
    if written.len() != entries.len() {
        return None;
    }
    let [(key, entry)] = written.as_slice() else {
        let code = if written.is_empty() {
            ModelCode::EmptyAlternative
        } else {
            ModelCode::AmbiguousAlternative
        };
        lowering.report(
            code,
            item.position(),
            path,
            format!(
                "a `multiplication` factor writes exactly one of {}",
                render_admitted(FACTOR_KEYS)
            ),
        );
        return None;
    };
    let value = entry.value();
    let value_path = path.field(key.as_str());
    if key.as_str() == "path" {
        return lowering.path(value, &value_path).map(Factor::Path);
    }
    // NOTE: the railroad (`omop_railroad.png`, CDM FIELD clause) gives `code` no
    // type; no specification governs this: our own design, an integer literal.
    let number = match *value.value() {
        MappingValue::Signed(number) => Some(number),
        MappingValue::Unsigned(number) => i64::try_from(number).ok(),
        _ => {
            lowering.wrong_kind(value, &value_path, ValueKind::Number);
            return None;
        }
    };
    let Some(number) = number else {
        lowering.report(
            ModelCode::InvalidFactor,
            value.position(),
            &value_path,
            "a `multiplication` `code` factor is a 64-bit signed integer",
        );
        return None;
    };
    Some(Factor::Code(Located::new(value.position(), number)))
}

/// Lowers an `Include`.
fn lower_include(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<Include> {
    lowering.refuse_unknown_keys(node, path, INCLUDE_KEYS, "an `Include`");
    let known = node.as_mapping().is_some_and(|entries| {
        entries
            .keys()
            .all(|key| INCLUDE_KEYS.contains(&key.as_str()))
    });
    let id_path = path.field("archetype_id");
    let archetype_id = lowering
        .required(node, path, "archetype_id")
        .and_then(|entry| {
            let value = entry.value();
            let text = lowering.text(value, &id_path)?;
            match ArchetypeId::from_str(text) {
                Ok(id) => Some(Located::new(value.position(), id)),
                Err(error) => {
                    let diagnostic = Diagnostic::error(
                        lowering.file.clone(),
                        openehr_mapping_core::diagnostic::DiagnosticCode::InvalidArchetypeId,
                        format!("`archetype_id` is not an archetype id: {error}"),
                    );
                    lowering.push(diagnostic, value.position(), &id_path);
                    None
                }
            }
        });
    let base_path = match node.get("base_path") {
        Some(value) => Some(lowering.path(value, &path.field("base_path"))?),
        None => None,
    };
    let archetype_id = archetype_id?;
    known.then_some(Include {
        position: node.position(),
        archetype_id,
        base_path,
    })
}

/// Lowers a `CustomMapping`.
fn lower_custom_mapping(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<CustomMapping> {
    lowering.refuse_unknown_keys(node, path, CUSTOM_MAPPING_KEYS, "a `CustomMapping`");
    let known = node.as_mapping().is_some_and(|entries| {
        entries
            .keys()
            .all(|key| CUSTOM_MAPPING_KEYS.contains(&key.as_str()))
    });
    let name_path = path.field("name");
    let name = lowering.required(node, path, "name").and_then(|entry| {
        let value = entry.value();
        let text = lowering.text(value, &name_path)?;
        Some(Located::new(value.position(), text.to_owned()))
    })?;
    known.then_some(CustomMapping {
        position: node.position(),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::GRAMMAR_VERSION;

    #[test]
    fn the_grammar_version_is_the_crate_grammar() {
        assert_eq!(format!("OMOCL/{GRAMMAR_VERSION}"), crate::GRAMMAR);
    }
}
