// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `spec` block, the preprocessors and the context block of a file.

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;
use openehr_mapping_core::value::ValueKind;

use crate::model::ast::Hierarchy;
use crate::model::ast::HierarchySplit;
use crate::model::ast::MappingContext;
use crate::model::ast::ModelSpec;
use crate::model::ast::Preprocessor;
use crate::model::ast::ProfileRef;
use crate::model::ast::SplitTarget;
use crate::model::ast::TemplateRef;
use crate::model::ast::keyword::Direction;
use crate::model::error::ModelCode;

use crate::model::parse::CONTEXT_KEYS;
use crate::model::parse::FHIR_CONFIG_KEYS;
use crate::model::parse::HIERARCHY_KEYS;
use crate::model::parse::Lowering;
use crate::model::parse::OPENEHR_CONFIG_KEYS;
use crate::model::parse::PREPROCESSOR_KEYS;
use crate::model::parse::PROFILE_KEYS;
use crate::model::parse::SPEC_KEYS;
use crate::model::parse::SPLIT_KEYS;
use crate::model::parse::SPLIT_TARGET_KEYS;
use crate::model::parse::TEMPLATE_KEYS;
use crate::model::parse::mapping::lower_condition;
use crate::model::parse::mapping::lower_with;

/// Reads the FHIRconnect half of the `spec` block.
pub(super) fn lower_spec(
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
pub(super) fn lower_preprocessor(
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

/// Reads the `context` block of a context mapping file.
pub(super) fn lower_context_block(
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
