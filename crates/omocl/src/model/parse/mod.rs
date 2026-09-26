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

mod alternative;
mod entity;
mod include;

use core::str::FromStr;
use std::collections::BTreeMap;
use std::path::PathBuf;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::grammar::GrammarSemVer;
use openehr_mapping_core::header::grammar::MappingLanguage;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::header::metadata::MappingType;
use openehr_mapping_core::loader::MappingDocument;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::value::MappingEntry;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::CdmSystem;
use crate::model::ast::CdmVersion;
use crate::model::ast::ConceptId;
use crate::model::ast::Entity;
use crate::model::ast::MappingFile;
use crate::model::ast::Spec;
use crate::model::error::ModelCode;
use crate::model::parse::entity::lower_entity;

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

#[cfg(test)]
mod tests {
    use super::GRAMMAR_VERSION;

    #[test]
    fn the_grammar_version_is_the_crate_grammar() {
        assert_eq!(format!("OMOCL/{GRAMMAR_VERSION}"), crate::GRAMMAR);
    }
}
