// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One legacy segment and its fields.

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::legacy::lower::LegacyDefect;
use crate::v2::legacy::lower::LegacyError;
use crate::v2::legacy::lower::invalid;
use crate::v2::legacy::lower::tolerate;
use crate::v2::legacy::source::{DataElementRow, FieldRow, VersionTables};
use crate::v2::lower::{Cardinality, Field, Length, Optionality, Segment, TABLE_VALUE_SET, Table};

fn number(file: &str, row: &str, text: &str) -> Result<u32, LegacyError> {
    text.parse::<u32>().map_err(|source| LegacyError::Number {
        file: file.to_owned(),
        row: row.to_owned(),
        value: text.to_owned(),
        source,
    })
}

pub(super) fn cardinality(
    file: &str,
    row: &str,
    min: u32,
    max: &str,
) -> Result<Cardinality, LegacyError> {
    let max = if max == "*" {
        None
    } else {
        Some(number(file, row, max)?)
    };
    if max.is_some_and(|max| min > max) {
        return Err(invalid(file, row, format!("min {min} above max {max:?}")));
    }
    Ok(Cardinality { min, max })
}

pub(super) fn lower_segment(
    version: &VersionTables,
    id: &str,
    elements: &BTreeMap<&str, &DataElementRow>,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<Segment, LegacyError> {
    let fields_file = version.file("fields");
    let Some(row) = version.segments.iter().find(|segment| segment.id == id) else {
        return Err(invalid(
            &version.file("segments"),
            id,
            "a tree names a segment with no row",
        ));
    };
    let mut rows: Vec<(u16, &FieldRow)> = Vec::new();
    for field in version.fields.iter().filter(|field| field.segment_id == id) {
        let label = format!("{id}.{}", field.position);
        let position = number(&fields_file, &label, &field.position)?;
        let position = u16::try_from(position).map_err(|source| LegacyError::Range {
            file: fields_file.clone(),
            row: label.clone(),
            source,
        })?;
        rows.push((position, field));
    }
    rows.sort_by_key(|(position, _)| *position);
    if rows.is_empty() {
        tolerate(hits, &fields_file, id, LegacyDefect::SegmentWithoutFields)?;
    }
    let mut fields: Vec<Field> = Vec::with_capacity(rows.len());
    for (position, field) in rows {
        let label = format!("{id}.{position}");
        while usize::from(position) > fields.len().saturating_add(1) {
            let missing = u16::try_from(fields.len().saturating_add(1)).map_err(|source| {
                LegacyError::Range {
                    file: fields_file.clone(),
                    row: label.clone(),
                    source,
                }
            })?;
            tolerate(hits, &fields_file, &label, LegacyDefect::MissingField)?;
            fields.push(Field {
                id: format!("{id}.{missing}"),
                position: missing,
                name: String::new(),
                data_type: None,
                cardinality: Cardinality {
                    min: 0,
                    max: Some(1),
                },
                optionality: Optionality::Unstated,
                length: None,
                conformance_length: None,
                table: None,
                standards_status: None,
            });
        }
        let index = fields.len();
        if usize::from(position) != index.saturating_add(1) {
            return Err(invalid(
                &fields_file,
                &label,
                format!(
                    "position {position} where {} belongs",
                    index.saturating_add(1)
                ),
            ));
        }
        let Some(element) = elements.get(field.data_element_id.as_str()) else {
            return Err(invalid(
                &fields_file,
                &label,
                format!("data element {} has no row", field.data_element_id),
            ));
        };
        if element.max_length == "." {
            tolerate(
                hits,
                &version.file("data_elements"),
                &element.id,
                LegacyDefect::DotLength,
            )?;
        }
        let lowered = lower_field(version, label, position, field, element)?;
        fields.push(stray_required(version, lowered, hits)?);
    }
    Ok(Segment {
        id: id.to_owned(),
        url: None,
        name: row.description.clone(),
        fields,
    })
}

/// The field as emitted: `C` where [`LegacyDefect::StrayRequired`] lists the
/// field's `R` as an export defect, refusing a listed field that is not `R`.
fn stray_required(
    version: &VersionTables,
    mut field: Field,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<Field, LegacyError> {
    let fields_file = version.file("fields");
    let stray = format!("{fields_file}#{}", field.id);
    if !LegacyDefect::StrayRequired
        .tolerated_in()
        .contains(&stray.as_str())
    {
        return Ok(field);
    }
    if field.optionality != Optionality::R {
        return Err(invalid(
            &fields_file,
            &field.id,
            "listed as a stray R but not R",
        ));
    }
    tolerate(hits, &stray, &field.id, LegacyDefect::StrayRequired)?;
    field.optionality = Optionality::C;
    Ok(field)
}

/// One field of a legacy segment, from its `fields.json` row and the data
/// element that row names.
fn lower_field(
    version: &VersionTables,
    label: String,
    position: u16,
    field: &FieldRow,
    element: &DataElementRow,
) -> Result<Field, LegacyError> {
    let fields_file = version.file("fields");
    let data_elements_file = version.file("data_elements");
    let min = number(&fields_file, &label, &field.min)?;
    let cardinality = cardinality(&fields_file, &label, min, &field.max)?;
    let optionality = match field.usage.as_str() {
        "R" => Optionality::R,
        "O" => Optionality::O,
        "C" => Optionality::C,
        "X" => Optionality::X,
        "B" => Optionality::B,
        "W" => Optionality::W,
        "NA" => Optionality::Na,
        other => {
            return Err(invalid(&fields_file, &label, format!("usage {other:?}")));
        }
    };
    let data_type = match element.datatype_id.as_str() {
        "-" => None,
        "" => return Err(invalid(&data_elements_file, &element.id, "no data type")),
        code => Some(code.to_owned()),
    };
    let length = match element.max_length.as_str() {
        "" | "." if element.min_length == 0 => None,
        "" | "." => Some(Length {
            min: element.min_length,
            max: None,
        }),
        text => Some(Length {
            min: element.min_length,
            max: Some(number(&data_elements_file, &element.id, text)?),
        }),
    };
    let table = element.table_id.as_ref().map(|table| Table {
        id: table.clone(),
        value_set: format!("{TABLE_VALUE_SET}{table}"),
    });
    Ok(Field {
        id: label,
        position,
        name: element.description.clone(),
        data_type,
        cardinality,
        optionality,
        length,
        conformance_length: None,
        table,
        standards_status: None,
    })
}

/// Whether a legacy segment's fields agree with a v2.9.1 segment's in
/// everything a parser reads: the count and, per position, the data type
/// code, the cardinality, the optionality and the table.
pub(super) fn same_table(legacy: &Segment, defined: &Segment) -> bool {
    legacy.fields.len() == defined.fields.len()
        && legacy
            .fields
            .iter()
            .zip(&defined.fields)
            .all(|(left, right)| {
                left.position == right.position
                    && left.data_type == right.data_type
                    && left.cardinality == right.cardinality
                    && left.optionality == right.optionality
                    && left.table.as_ref().map(|table| &table.id)
                        == right.table.as_ref().map(|table| &table.id)
            })
}
