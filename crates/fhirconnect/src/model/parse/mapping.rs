// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `mappings` list: one mapping, its `with`, its `followedBy`, its
//! concept keys, its `manual` entries and its conditions.

use core::str::FromStr;

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::Condition;
use crate::model::ast::FollowedBy;
use crate::model::ast::LinkMapping;
use crate::model::ast::ManualEntry;
use crate::model::ast::ManualPath;
use crate::model::ast::Mapping;
use crate::model::ast::ReferenceMapping;
use crate::model::ast::With;
use crate::model::ast::keyword::ConditionOperator;
use crate::model::ast::keyword::DataType;
use crate::model::ast::keyword::Direction;
use crate::model::ast::keyword::ExtensionMethod;
use crate::model::error::ModelCode;

use crate::model::parse::CONDITION_KEYS;
use crate::model::parse::LINK_KEYS;
use crate::model::parse::Lowering;
use crate::model::parse::MANUAL_KEYS;
use crate::model::parse::MANUAL_PATH_KEYS;
use crate::model::parse::MAPPING_KEYS;
use crate::model::parse::WITH_KEYS;

/// Reads the `mappings` sequence, refusing a `mappings` written with no value.
pub(super) fn lower_mappings(
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
        (Some(mut written), Some(data_type)) => {
            match written.data_type {
                Some(ref inner) if inner.value() != data_type.value() => lowering.report(
                    ModelCode::ConflictingDataType,
                    data_type.position(),
                    &path.field("type"),
                    format!(
                        "the mapping writes the data type `{}` and its `with` writes `{}`",
                        data_type.value(),
                        inner.value()
                    ),
                ),
                Some(_) => {}
                None => written.data_type = Some(data_type),
            }
            Some(written)
        }
        (None, Some(data_type)) => Some(With {
            position: data_type.position(),
            fhir: None,
            openehr: None,
            data_type: Some(data_type),
            value: None,
        }),
        (written, None) => written,
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
pub(super) fn lower_with(
    lowering: &mut Lowering,
    node: &PositionedValue,
    path: &ModelPath,
) -> Option<With> {
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
pub(super) fn lower_condition(
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
    // NOTE: `Conditions.adoc` §type says the condition is an array, which the
    // published schema and every published file contradict, so one object per
    // key is what this reads (reported on issue #182).
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
