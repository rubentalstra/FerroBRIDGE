// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowering of an `Include` and a custom mapping.

use core::str::FromStr;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::archetype::ArchetypeId;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::value::PositionedValue;

use crate::model::ast::CustomMapping;
use crate::model::ast::Include;
use crate::model::parse::CUSTOM_MAPPING_KEYS;
use crate::model::parse::INCLUDE_KEYS;
use crate::model::parse::Lowering;

/// Lowers an `Include`.
pub(super) fn lower_include(
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
pub(super) fn lower_custom_mapping(
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
