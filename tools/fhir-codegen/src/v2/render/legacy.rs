// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `legacy` module tree: one module per version and its data types.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::v2::legacy::lower::{LegacyModel, LegacyVersion, Owner};
use crate::v2::render::RenderError;
use crate::v2::render::index::Index;
use crate::v2::render::index::render_index;
use crate::v2::render::legacy_banner;
use crate::v2::render::module_names;
use crate::v2::render::names;
use crate::v2::render::segment::render_segment;
use crate::v2::render::structure::render_linked_structure;
use crate::v2::render::structure::render_structure;
use crate::v2::render::version_module;

/// The segment and data type modules of one legacy version.
fn render_legacy_segments(
    banner: &str,
    version: &LegacyVersion,
    module: &str,
    segment_names: &BTreeMap<String, (String, String)>,
    data_types: &BTreeMap<String, (String, String)>,
    files: &mut BTreeMap<String, String>,
) -> Result<(), RenderError> {
    if !version.segments.is_empty() {
        let segment_doc = format!(
            "The segments of the {} tables that no v2.9.1 or earlier `static` carries, one module each.",
            version.version
        );
        files.insert(
            format!("legacy/{module}/segment/mod.rs"),
            render_index(
                banner,
                segment_names,
                Index::Segment,
                Some((&segment_doc, "ORC")),
            )?,
        );
    }
    let legacy_types = module_names(version.data_types.keys())?;
    if !version.data_types.is_empty() {
        files.insert(
            format!("legacy/{module}/data_type.rs"),
            render_legacy_data_types(banner, version, &legacy_types)?,
        );
    }
    let tables = LegacyTables {
        version: &version.version,
        module,
        data_types: &legacy_types,
    };
    for (id, segment) in &version.segments {
        let (file, name) = names(id);
        files.insert(
            format!("legacy/{module}/segment/{file}.rs"),
            render_segment(banner, &name, segment, data_types, Some(&tables))?,
        );
    }
    Ok(())
}

/// Every file under `legacy/`: the version index, and per version its
/// structure, segment and data type modules.
pub(super) fn render_legacy(
    legacy: &LegacyModel,
    current_segments: &BTreeMap<String, (String, String)>,
    data_types: &BTreeMap<String, (String, String)>,
    files: &mut BTreeMap<String, String>,
) -> Result<(), RenderError> {
    let banner = legacy_banner(legacy);
    let mut index: Vec<(&str, usize, String)> = Vec::new();
    for (order, version) in legacy.versions.iter().enumerate() {
        let module = version_module(&version.version);
        let structure_names = module_names(version.structures.keys())?;
        let segment_names = module_names(version.segments.keys())?;
        files.insert(
            format!("legacy/{module}/mod.rs"),
            render_version_module(&banner, version)?,
        );
        let structure_doc = format!(
            "The legacy message structures of the {} tables, one module each.",
            version.version
        );
        files.insert(
            format!("legacy/{module}/structure/mod.rs"),
            render_index(
                &banner,
                &structure_names,
                Index::Structure,
                Some((&structure_doc, "ORM_O01")),
            )?,
        );
        render_legacy_segments(&banner, version, &module, &segment_names, data_types, files)?;
        let resolve = |id: &str| match version.links.get(id) {
            Some(Owner::Current) => current_segments
                .get(id)
                .map(|(file, name)| format!("crate::segment::{file}::{name}")),
            Some(Owner::Version(owner)) => {
                let (file, name) = names(id);
                Some(format!(
                    "crate::legacy::{}::segment::{file}::{name}",
                    version_module(owner)
                ))
            }
            None => segment_names
                .get(id)
                .map(|(file, name)| format!("crate::legacy::{module}::segment::{file}::{name}")),
        };
        for (id, structure) in &version.structures {
            let (file, name) = names(id);
            let text = match version.trees.get(id) {
                None => render_structure(&banner, &name, structure, &resolve, false)?,
                Some(owner) => render_linked_structure(&banner, &name, structure, owner)?,
            };
            files.insert(format!("legacy/{module}/structure/{file}.rs"), text);
            index.push((
                id.as_str(),
                order,
                format!("{module}::structure::{file}::{name}"),
            ));
        }
    }
    index.sort_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)));
    let mut out = banner;
    out.push_str(
        "//! The message structures of the tables of each earlier HL7 v2 version, one\n//! module per version, those the v2.9.1 definitions no longer carry among them.\n//!\n//! A version module holds `structure`, one module per structure with its\n//! tree, and `segment` for each segment no other `static` carries. A tree\n//! links a segment whose field table agrees with the v2.9.1 one to the v2.9.1\n//! `static`, and one identical to an earlier version's to that version's; a\n//! structure whose tree is identical to the v2.9.1 tree or to an earlier\n//! version's links its nodes to that tree. `data_type` holds the data type\n//! codes the version's own segments name, each with the base type a\n//! version-specific code stands for. Each [`crate::model::Structure`] here\n//! carries its version and, for a structure the v2.9.1 definitions no longer\n//! carry, the version it is withdrawn as of.\n\n",
    );
    for version in &legacy.versions {
        writeln!(out, "pub mod {};", version_module(&version.version))?;
    }
    writeln!(
        out,
        "\n/// Every legacy message structure, in `(id, version)` order with the\n/// versions ordered numerically.\npub static STRUCTURES: [&crate::model::Structure; {}] = [",
        index.len()
    )?;
    for (_, _, path) in &index {
        writeln!(out, "    &{path},")?;
    }
    out.push_str(
        "];\n\n/// Returns every version's structure whose id is `id`, for example\n/// `ORM_O01`, in version order.\n#[must_use]\npub fn versions(id: &str) -> &'static [&'static crate::model::Structure] {\n    let start = STRUCTURES.partition_point(|entry| entry.id < id);\n    let end = STRUCTURES.partition_point(|entry| entry.id <= id);\n    STRUCTURES.get(start..end).unwrap_or(&[])\n}\n\n/// Returns the structure `id` of the tables of `version`, for example\n/// `ORM_O01` of `2.5.1`.\n#[must_use]\npub fn find(id: &str, version: &str) -> Option<&'static crate::model::Structure> {\n    versions(id)\n        .iter()\n        .copied()\n        .find(|entry| entry.version == version)\n}\n",
    );
    files.insert(String::from("legacy/mod.rs"), out);
    Ok(())
}

fn render_version_module(banner: &str, version: &LegacyVersion) -> Result<String, RenderError> {
    let mut out = String::from(banner);
    write!(
        out,
        "//! The message structures of the {} tables, and the segments they name\n//! that no v2.9.1 or earlier `static` carries.\n\n",
        version.version
    )?;
    if !version.data_types.is_empty() {
        out.push_str("pub mod data_type;\n");
    }
    if !version.segments.is_empty() {
        out.push_str("pub mod segment;\n");
    }
    out.push_str("pub mod structure;\n");
    Ok(out)
}

/// The data type codes of one legacy version, from a segment module.
#[derive(Debug, Clone, Copy)]
pub(super) struct LegacyTables<'a> {
    pub(super) version: &'a str,
    pub(super) module: &'a str,
    pub(super) data_types: &'a BTreeMap<String, (String, String)>,
}

/// The `data_type` module of a legacy version: one `static` per data type
/// code its segments' fields name.
fn render_legacy_data_types(
    banner: &str,
    version: &LegacyVersion,
    names: &BTreeMap<String, (String, String)>,
) -> Result<String, RenderError> {
    let mut out = String::from(banner);
    let uses = if version
        .data_types
        .values()
        .any(|data_type| data_type.base.is_some())
    {
        "use crate::model::{LegacyBase, LegacyDataType};\n"
    } else {
        "use crate::model::LegacyDataType;\n"
    };
    write!(
        out,
        "//! The data type codes the fields of the {} tables name, with the base\n//! type each version-specific code stands for.\n\n{uses}",
        version.version
    )?;
    for ((code, data_type), (_, name)) in version.data_types.iter().zip(names.values()) {
        let base = data_type.base.as_ref().map_or_else(
            || String::from("None"),
            |base| {
                let table = base
                    .table
                    .as_ref()
                    .map_or_else(|| String::from("None"), |table| format!("Some({table:?})"));
                format!(
                    "Some(LegacyBase {{ code: {:?}, table: {table} }})",
                    base.code
                )
            },
        );
        write!(
            out,
            "\n/// The `{code}` data type code of the {} tables.\npub static {name}: LegacyDataType = LegacyDataType {{\n    code: {code:?},\n    version: {:?},\n    name: {:?},\n    base: {base},\n}};\n",
            version.version, version.version, data_type.name
        )?;
    }
    Ok(out)
}
