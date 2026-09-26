// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowering of one column alternative and the forms it writes.

use core::str::FromStr;
use std::collections::BTreeMap;

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::value::MappingEntry;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::Alternative;
use crate::model::ast::AtCode;
use crate::model::ast::ConceptMap;
use crate::model::ast::Factor;
use crate::model::ast::Multiplication;
use crate::model::error::ModelCode;
use crate::model::parse::ALTERNATIVE_KEYS;
use crate::model::parse::CONCEPT_MAP_KEYS;
use crate::model::parse::FACTOR_KEYS;
use crate::model::parse::Lowering;
use crate::model::parse::render_admitted;

/// Lowers one alternative, which writes exactly one of the four forms.
pub(super) fn lower_alternative(
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
pub(super) fn lower_concept_map(
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
pub(super) fn lower_multiplication(
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
pub(super) fn lower_factor(
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
