// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Segments and their field tables.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::v2::lower::{
    Cardinality, ConditionalCode, ConformanceLength, Field, Length, Optionality, Segment,
    StandardsStatus, Table,
};
use crate::v2::render::RenderError;
use crate::v2::render::legacy::LegacyTables;

pub(super) fn cardinality(cardinality: Cardinality) -> String {
    let max = match cardinality.max {
        Some(max) => format!("Max::Bounded({max})"),
        None => String::from("Max::Unbounded"),
    };
    format!("Cardinality {{ min: {}, max: {max} }}", cardinality.min)
}

pub(super) fn optionality(value: Optionality, uses: &mut BTreeSet<&'static str>) -> String {
    let code = |code: ConditionalCode| match code {
        ConditionalCode::R => "ConditionalCode::R",
        ConditionalCode::Re => "ConditionalCode::Re",
        ConditionalCode::O => "ConditionalCode::O",
        ConditionalCode::X => "ConditionalCode::X",
    };
    match value {
        Optionality::R => String::from("Optionality::R"),
        Optionality::Re => String::from("Optionality::Re"),
        Optionality::O => String::from("Optionality::O"),
        Optionality::C => String::from("Optionality::C"),
        Optionality::Conditional(first, second) => {
            uses.insert("ConditionalCode");
            format!(
                "Optionality::Conditional {{ first: {}, second: {} }}",
                code(first),
                code(second)
            )
        }
        Optionality::X => String::from("Optionality::X"),
        Optionality::B => String::from("Optionality::B"),
        Optionality::W => String::from("Optionality::W"),
        Optionality::Na => String::from("Optionality::Na"),
        Optionality::Unstated => String::from("Optionality::Unstated"),
    }
}

fn optional_u32(value: Option<u32>) -> String {
    value.map_or_else(|| String::from("None"), |value| format!("Some({value})"))
}

pub(super) fn length(value: Option<Length>, uses: &mut BTreeSet<&'static str>) -> String {
    value.map_or_else(
        || String::from("None"),
        |length| {
            uses.insert("Length");
            format!(
                "Some(Length {{ min: {}, max: {} }})",
                length.min,
                optional_u32(length.max)
            )
        },
    )
}

pub(super) fn conformance_length(
    value: Option<ConformanceLength>,
    uses: &mut BTreeSet<&'static str>,
) -> String {
    value.map_or_else(
        || String::from("None"),
        |value| {
            uses.insert("ConformanceLength");
            let no_truncate = value
                .no_truncate
                .map_or_else(|| String::from("None"), |flag| format!("Some({flag})"));
            format!(
                "Some(ConformanceLength {{ length: {}, no_truncate: {no_truncate} }})",
                optional_u32(value.length)
            )
        },
    )
}

pub(super) fn table(value: Option<&Table>, uses: &mut BTreeSet<&'static str>) -> String {
    value.map_or_else(
        || String::from("None"),
        |table| {
            uses.insert("Table");
            format!(
                "Some(Table {{ id: {:?}, value_set: {:?} }})",
                table.id, table.value_set
            )
        },
    )
}

/// The path of the data type static `code` names, from a sibling module.
pub(super) fn data_type_path(
    code: &str,
    data_types: &BTreeMap<String, (String, String)>,
) -> Option<String> {
    data_types
        .get(code)
        .map(|(module, name)| format!("&data_type::{module}::{name}"))
}

pub(super) fn field(
    field: &Field,
    data_types: &BTreeMap<String, (String, String)>,
    legacy: Option<&LegacyTables<'_>>,
    uses: &mut BTreeSet<&'static str>,
) -> Result<String, RenderError> {
    let data_type = match (&field.data_type, legacy) {
        (None, _) => String::from("None"),
        (Some(code), Some(tables)) => {
            uses.insert("DataTypeRef");
            let Some((_, name)) = tables.data_types.get(code) else {
                return Err(RenderError::MissingDataType {
                    component: field.id.clone(),
                    data_type: code.clone(),
                });
            };
            format!(
                "Some(DataTypeRef::Legacy(&crate::legacy::{}::data_type::{name}))",
                tables.module
            )
        }
        (Some(code), None) => {
            uses.insert("DataTypeRef");
            match data_type_path(code, data_types) {
                Some(path) => {
                    uses.insert("data_type");
                    format!("Some(DataTypeRef::Defined({path}))")
                }
                None => format!("Some(DataTypeRef::Undefined({code:?}))"),
            }
        }
    };
    let length = length(field.length, uses);
    let conformance_length = conformance_length(field.conformance_length, uses);
    let table = table(field.table.as_ref(), uses);
    let standards_status = field.standards_status.map_or_else(
        || String::from("None"),
        |status| {
            uses.insert("StandardsStatus");
            match status {
                StandardsStatus::Deprecated => String::from("Some(StandardsStatus::Deprecated)"),
                StandardsStatus::Withdrawn => String::from("Some(StandardsStatus::Withdrawn)"),
            }
        },
    );
    Ok(format!(
        "Field {{ id: {:?}, position: {}, name: {:?}, data_type: {data_type}, cardinality: {}, optionality: {}, length: {length}, conformance_length: {conformance_length}, table: {table}, standards_status: {standards_status} }}",
        field.id,
        field.position,
        field.name,
        cardinality(field.cardinality),
        optionality(field.optionality, uses),
    ))
}

/// The `use` lines for `uses`: the model shapes, and the `data_type`
/// module when a static names one.
pub(super) fn uses_line(uses: &BTreeSet<&'static str>) -> String {
    let names: Vec<&str> = uses
        .iter()
        .copied()
        .filter(|name| *name != "data_type")
        .collect();
    let mut out = format!("use crate::model::{{{}}};\n", names.join(", "));
    if uses.contains("data_type") {
        out.push_str("use crate::data_type;\n");
    }
    out
}

pub(super) fn render_segment(
    banner: &str,
    name: &str,
    segment: &Segment,
    data_types: &BTreeMap<String, (String, String)>,
    legacy: Option<&LegacyTables<'_>>,
) -> Result<String, RenderError> {
    let mut uses: BTreeSet<&'static str> = ["Segment"].into();
    let mut fields = String::new();
    for entry in &segment.fields {
        uses.extend(["Cardinality", "Field", "Max", "Optionality"]);
        writeln!(
            fields,
            "        {},",
            field(entry, data_types, legacy, &mut uses)?
        )?;
    }
    let legacy = legacy.map(|tables| tables.version);
    let mut out = String::from(banner);
    writeln!(
        out,
        "//! The `{}` segment: {}.\n",
        segment.id,
        segment.name.trim_end_matches('.')
    )?;
    out.push_str(&uses_line(&uses));
    let (doc, url) = match (legacy, &segment.url) {
        (Some(version), _) => (
            format!("The `{}` segment of the {version} tables.", segment.id),
            String::from("None"),
        ),
        (None, Some(url)) => (
            format!("The `{}` segment definition, `{url}`.", segment.id),
            format!("Some({url:?})"),
        ),
        (None, None) => (
            format!("The `{}` segment definition.", segment.id),
            String::from("None"),
        ),
    };
    writeln!(
        out,
        "\n/// {doc}\npub static {name}: Segment = Segment {{\n    id: {:?},\n    url: {url},\n    name: {:?},\n    fields: &[\n{fields}    ],\n}};",
        segment.id, segment.name
    )?;
    Ok(out)
}
