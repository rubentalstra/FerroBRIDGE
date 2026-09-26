// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The structure and message indexes.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::v2::legacy::lower::LegacyModel;
use crate::v2::lower::Model;
use crate::v2::render::RenderError;
use crate::v2::render::version_module;

#[derive(Debug, Clone, Copy)]
pub(super) enum Index {
    Segment,
    Structure,
    DataType,
}

pub(super) fn render_index(
    banner: &str,
    names: &BTreeMap<String, (String, String)>,
    index: Index,
    legacy: Option<(&str, &str)>,
) -> Result<String, RenderError> {
    let (doc, kind, list, noun, key, example) = match index {
        Index::Segment => (
            "Every segment definition, one module each.",
            "Segment",
            "SEGMENTS",
            "segment",
            "definition id",
            "OBX",
        ),
        Index::Structure => (
            "Every message structure of the HL7 v2 definitions, one module each.",
            "Structure",
            "STRUCTURES",
            "message structure",
            "definition id",
            "ORU_R01-A",
        ),
        Index::DataType => (
            "Every primitive and complex data type of the HL7 v2 definitions, one module each.",
            "DataType",
            "DATA_TYPES",
            "data type",
            "code",
            "CX",
        ),
    };
    let field = match index {
        Index::Segment | Index::Structure => "id",
        Index::DataType => "code",
    };
    let (doc, example) = legacy.unwrap_or((doc, example));
    let mut out = String::from(banner);
    writeln!(out, "//! {doc}\n")?;
    for (module, _) in names.values() {
        writeln!(out, "pub mod {module};")?;
    }
    writeln!(
        out,
        "\n/// Every {noun}, in definition id order.\npub static {list}: [&crate::model::{kind}; {}] = [",
        names.len()
    )?;
    for (module, name) in names.values() {
        writeln!(out, "    &{module}::{name},")?;
    }
    writeln!(
        out,
        "];\n\n/// Returns the {noun} whose {key} is `{field}`, for example `{example}`.\n#[must_use]\npub fn find({field}: &str) -> Option<&'static crate::model::{kind}> {{\n    match {list}.binary_search_by(|entry| entry.{field}.cmp({field})) {{\n        Ok(index) => {list}.get(index).copied(),\n        Err(_) => None,\n    }}\n}}"
    )?;
    Ok(out)
}

/// The message index: every message definition in `(code, event)` order.
pub(super) fn render_message_index(
    banner: &str,
    model: &Model,
    names: &BTreeMap<String, (String, String)>,
    legacy: &LegacyModel,
) -> Result<String, RenderError> {
    let mut out = String::from(banner);
    writeln!(
        out,
        "//! Every message definition of the HL7 v2 definitions, one module each,\n//! and the legacy message structures by code, event and version.\n"
    )?;
    for (module, _) in names.values() {
        writeln!(out, "pub mod {module};")?;
    }
    let mut ordered: Vec<(&str, &str, &(String, String))> = Vec::with_capacity(names.len());
    for (id, message) in &model.messages {
        if let Some(entry) = names.get(id) {
            ordered.push((&message.code, &message.event, entry));
        }
    }
    ordered.sort_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)));
    writeln!(
        out,
        "\n/// Every message definition, in `(code, event)` order.\npub static MESSAGES: [&crate::model::Message; {}] = [",
        ordered.len()
    )?;
    for (_, _, (module, name)) in &ordered {
        writeln!(out, "    &{module}::{name},")?;
    }
    writeln!(
        out,
        "];\n\n/// Returns the message definition sent as `code^event` (`MSH-9.1` and\n/// `MSH-9.2`), for example `ORU` and `R01`.\n#[must_use]\npub fn find(code: &str, event: &str) -> Option<&'static crate::model::Message> {{\n    match MESSAGES.binary_search_by(|entry| (entry.code, entry.event).cmp(&(code, event))) {{\n        Ok(index) => MESSAGES.get(index).copied(),\n        Err(_) => None,\n    }}\n}}"
    )?;
    writeln!(
        out,
        "\n/// Every legacy message structure by the message code and trigger event its\n/// tables give it, in `(code, event, version)` order with the versions\n/// ordered numerically.\npub static LEGACY: [crate::model::LegacyMessage; {}] = [",
        legacy.messages.len()
    )?;
    for entry in &legacy.messages {
        let Some(version) = legacy
            .versions
            .iter()
            .find(|version| version.version == entry.version)
        else {
            return Err(RenderError::MissingStructure {
                message: format!("{}^{} {}", entry.code, entry.event, entry.version),
                structure: entry.structure.clone(),
            });
        };
        if !version.structures.contains_key(&entry.structure) {
            return Err(RenderError::MissingStructure {
                message: format!("{}^{} {}", entry.code, entry.event, entry.version),
                structure: entry.structure.clone(),
            });
        }
        let (file, name) = crate::v2::render::names(&entry.structure);
        writeln!(
            out,
            "    crate::model::LegacyMessage {{ code: {:?}, event: {:?}, version: {:?}, structure: &crate::legacy::{}::structure::{file}::{name} }},",
            entry.code,
            entry.event,
            entry.version,
            version_module(&entry.version)
        )?;
    }
    out.push_str(
        "];\n\n/// Returns the legacy entries sent as `code^event` (`MSH-9.1` and\n/// `MSH-9.2`), for example `ORM` and `O01`, one per version in version order.\n#[must_use]\npub fn legacy(code: &str, event: &str) -> &'static [crate::model::LegacyMessage] {\n    let start = LEGACY.partition_point(|entry| (entry.code, entry.event) < (code, event));\n    let end = LEGACY.partition_point(|entry| (entry.code, entry.event) <= (code, event));\n    LEGACY.get(start..end).unwrap_or(&[])\n}\n\n/// Returns the legacy entry sent as `code^event` in the tables of `version`\n/// (`MSH-12`), for example `ORM`, `O01` and `2.5.1`.\n#[must_use]\npub fn find_legacy(\n    code: &str,\n    event: &str,\n    version: &str,\n) -> Option<&'static crate::model::LegacyMessage> {\n    legacy(code, event)\n        .iter()\n        .find(|entry| entry.version == version)\n}\n",
    );
    Ok(out)
}
