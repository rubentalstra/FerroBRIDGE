// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Data types and their components.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::v2::lower::{Component, DataType};
use crate::v2::render::RenderError;
use crate::v2::render::segment::cardinality;
use crate::v2::render::segment::conformance_length;
use crate::v2::render::segment::data_type_path;
use crate::v2::render::segment::length;
use crate::v2::render::segment::optionality;
use crate::v2::render::segment::table;
use crate::v2::render::segment::uses_line;

pub(super) fn component(
    component: &Component,
    data_types: &BTreeMap<String, (String, String)>,
    uses: &mut BTreeSet<&'static str>,
) -> Result<String, RenderError> {
    let data_type = match &component.data_type {
        None => String::from("None"),
        Some(code) => {
            let Some(path) = data_type_path(code, data_types) else {
                return Err(RenderError::MissingDataType {
                    component: component.id.clone(),
                    data_type: code.clone(),
                });
            };
            uses.insert("data_type");
            format!("Some({path})")
        }
    };
    let count = component.cardinality.map_or_else(
        || String::from("None"),
        |count| {
            uses.extend(["Cardinality", "Max"]);
            format!("Some({})", cardinality(count))
        },
    );
    let length = length(component.length, uses);
    let conformance_length = conformance_length(component.conformance_length, uses);
    let table = table(component.table.as_ref(), uses);
    Ok(format!(
        "Component {{ id: {:?}, position: {}, name: {:?}, data_type: {data_type}, cardinality: {count}, optionality: {}, length: {length}, conformance_length: {conformance_length}, table: {table} }}",
        component.id,
        component.position,
        component.name,
        optionality(component.optionality, uses),
    ))
}

pub(super) fn render_data_type(
    banner: &str,
    name: &str,
    data_type: &DataType,
    data_types: &BTreeMap<String, (String, String)>,
) -> Result<String, RenderError> {
    let mut uses: BTreeSet<&'static str> = ["DataType"].into();
    let mut components = String::new();
    for entry in &data_type.components {
        uses.extend(["Component", "Optionality"]);
        writeln!(
            components,
            "        {},",
            component(entry, data_types, &mut uses)?
        )?;
    }
    let kind = if data_type.components.is_empty() {
        "primitive"
    } else {
        "complex"
    };
    let mut out = String::from(banner);
    writeln!(
        out,
        "//! The `{}` {kind} data type: {}.\n",
        data_type.code,
        data_type.name.trim_end_matches('.')
    )?;
    out.push_str(&uses_line(&uses));
    let components = if components.is_empty() {
        String::from("&[]")
    } else {
        format!("&[\n{components}    ]")
    };
    writeln!(
        out,
        "\n/// The `{}` data type definition, `{}`.\npub static {name}: DataType = DataType {{\n    code: {:?},\n    url: {:?},\n    name: {:?},\n    components: {components},\n}};",
        data_type.code, data_type.url, data_type.code, data_type.url, data_type.name
    )?;
    Ok(out)
}
