// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowering of one `mappings` entry: its entity, record and columns.

use core::str::FromStr;

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::value::MappingEntry;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::Alternative;
use crate::model::ast::Column;
use crate::model::ast::Entity;
use crate::model::ast::EntityType;
use crate::model::ast::Record;
use crate::model::ast::Target;
use crate::model::error::ModelCode;
use crate::model::parse::COLUMN_KEYS;
use crate::model::parse::Lowering;
use crate::model::parse::alternative::lower_alternative;
use crate::model::parse::include::lower_custom_mapping;
use crate::model::parse::include::lower_include;
use crate::model::parse::render_admitted;
use crate::model::projection::key_without_column;
use crate::model::projection::projection;

/// Lowers one entry of `mappings`.
pub(super) fn lower_entity(
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
pub(super) fn lower_record(
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
pub(super) fn lower_column(
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
pub(super) fn lower_alternatives(
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
