// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The key-to-column projection against the CDM v5.4 column metadata that
//! `omop-cdm` generates from the OHDSI field definitions, and against the
//! authored schema.

use std::collections::BTreeSet;
use std::error::Error;

use omocl::model::ast::Target;
use omocl::model::projection::Fill;
use omocl::model::projection::KEYS_WITHOUT_COLUMN;
use omocl::model::projection::Part;
use omocl::model::projection::projection;
use omocl::model::schema::SCHEMA;

#[test]
fn every_projected_column_exists_in_its_cdm_table() -> Result<(), Box<dyn Error>> {
    for target in Target::ALL {
        let table = omop_cdm::meta::table(target.table())
            .ok_or_else(|| format!("CDM v5.4 has no `{}` table", target.table()))?;
        for key in projection(*target).keys {
            for column in key.columns {
                assert!(
                    table.column(column.column).is_some(),
                    "`{target}.{}` names `{}.{}`, which CDM v5.4 does not define",
                    key.key,
                    table.name,
                    column.column
                );
            }
        }
    }
    Ok(())
}

#[test]
fn every_key_lists_its_columns_in_cdm_definition_order() -> Result<(), Box<dyn Error>> {
    for target in Target::ALL {
        let table = omop_cdm::meta::table(target.table()).ok_or("a CDM table")?;
        for key in projection(*target).keys {
            let order: Vec<usize> = key
                .columns
                .iter()
                .filter_map(|column| {
                    table
                        .columns
                        .iter()
                        .position(|meta| meta.name == column.column)
                })
                .collect();
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(order, sorted, "`{target}.{}`", key.key);
        }
    }
    Ok(())
}

#[test]
fn no_two_keys_of_a_target_write_one_column_except_the_procedure_date_pair() {
    for target in Target::ALL {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for key in projection(*target).keys {
            for column in key.columns {
                if let Some((first, _)) = seen.iter().find(|(_, c)| *c == column.column) {
                    assert_eq!(
                        (*target, *first, key.key),
                        (
                            Target::ProcedureOccurrence,
                            "procedure_date",
                            "procedure_start_date"
                        ),
                        "`{}` is written by two keys",
                        column.column
                    );
                }
                seen.push((key.key, column.column));
            }
        }
    }
}

#[test]
fn a_key_filled_by_value_kind_offers_at_least_two_kinds() {
    for target in Target::ALL {
        for key in projection(*target).keys {
            if key.fill != Fill::OneByValueKind {
                continue;
            }
            let kinds = key
                .columns
                .iter()
                .filter(|column| matches!(column.part, Part::Number | Part::Concept | Part::Text))
                .count();
            assert!(kinds >= 2, "`{target}.{}` offers {kinds} kind", key.key);
        }
    }
}

#[test]
fn a_concept_key_writes_its_standard_and_its_source_concept() {
    let measurement = projection(Target::Measurement)
        .key("concept_id")
        .expect("Measurement admits concept_id");
    let columns: Vec<&str> = measurement.columns.iter().map(|c| c.column).collect();
    assert_eq!(
        columns,
        [
            "measurement_concept_id",
            "measurement_source_value",
            "measurement_source_concept_id"
        ]
    );
}

#[test]
fn the_value_key_reads_the_dv_quantity_projection() {
    let measurement = projection(Target::Measurement);
    for (key, column, part) in [
        ("value", "value_as_number", Part::Number),
        ("unit", "unit_concept_id", Part::Units),
        ("range_low", "range_low", Part::RangeLow),
        ("range_high", "range_high", Part::RangeHigh),
        ("operator_concept_id", "operator_concept_id", Part::Operator),
    ] {
        let projected = measurement.key(key).expect("the key is admitted");
        assert!(
            projected
                .columns
                .iter()
                .any(|c| c.column == column && c.part == part),
            "`{key}` does not write `{column}` as {part:?}"
        );
    }
}

#[test]
fn procedure_start_date_writes_the_cdm_procedure_date() {
    let key = projection(Target::ProcedureOccurrence)
        .key("procedure_start_date")
        .expect("the library key is admitted");
    let columns: Vec<&str> = key.columns.iter().map(|c| c.column).collect();
    assert_eq!(columns, ["procedure_date", "procedure_datetime"]);
}

#[test]
fn a_key_without_column_really_has_no_column() -> Result<(), Box<dyn Error>> {
    for entry in KEYS_WITHOUT_COLUMN {
        let table = omop_cdm::meta::table(entry.target.table()).ok_or("a CDM table")?;
        assert!(
            table
                .columns
                .iter()
                .all(|column| !column.name.starts_with(entry.key)),
            "`{}` has a `{}` column after all",
            table.name,
            entry.key
        );
        assert!(
            projection(entry.target).key(entry.key).is_none(),
            "`{}.{}` is both projected and recorded as without column",
            entry.target,
            entry.key
        );
    }
    Ok(())
}

#[test]
fn the_schema_admits_exactly_the_projected_keys_per_target() -> Result<(), Box<dyn Error>> {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA)?;
    for target in Target::ALL {
        let properties = schema
            .pointer(&format!("/definitions/{}/properties", target.as_str()))
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("the schema defines no `{target}`"))?;
        let admitted: BTreeSet<&str> = properties
            .keys()
            .map(String::as_str)
            .filter(|key| !matches!(*key, "type" | "base_path"))
            .collect();
        let projected: BTreeSet<&str> = projection(*target).keys.iter().map(|k| k.key).collect();
        assert_eq!(admitted, projected, "`{target}`");
    }
    Ok(())
}
