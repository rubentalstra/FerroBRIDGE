// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The generated table set and every table's columns against the two vendored
//! sources: `OMOP_CDMv5.4_Table_Level.csv` and OHDSI's rendered
//! `OMOPCDM_postgresql_5.4_ddl.sql`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;

/// The vendored table definitions, read from the repository rather than the
/// packaged crate.
const TABLE_LEVEL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/omop-cdm/inst/csv/OMOP_CDMv5.4_Table_Level.csv"
);

/// The number of tables the CDM v5.4 definitions carry across the `CDM`,
/// `VOCAB` and `RESULTS` schemas.
const TABLE_COUNT: usize = 39;

#[test]
fn the_generated_table_set_is_the_definitions_table_set() -> Result<(), Box<dyn Error>> {
    let csv = fs::read_to_string(TABLE_LEVEL)?;
    let defined = defined_tables(&csv);
    assert_eq!(
        TABLE_COUNT,
        defined.len(),
        "the vendored table definitions no longer carry {TABLE_COUNT} tables"
    );
    let generated: Vec<&str> = omop_cdm::generated::TABLES
        .iter()
        .map(|table| table.name)
        .collect();
    assert_eq!(
        defined, generated,
        "the generated tables and the table definitions disagree"
    );
    Ok(())
}

#[test]
fn every_generated_column_set_is_the_ddl_column_set() -> Result<(), Box<dyn Error>> {
    let ddl = ddl_columns(omop_cdm::generated::ddl::DDL);
    assert_eq!(
        TABLE_COUNT,
        ddl.len(),
        "the vendored DDL no longer creates {TABLE_COUNT} tables"
    );
    for table in &omop_cdm::generated::TABLES {
        let expected = ddl
            .get(table.name)
            .ok_or_else(|| format!("the DDL creates no {} table", table.name))?;
        let generated: Vec<&str> = table.columns.iter().map(|column| column.name).collect();
        assert_eq!(
            expected, &generated,
            "the {} columns and the DDL's disagree",
            table.name
        );
    }
    Ok(())
}

#[test]
fn the_capital_i_integer_of_the_definitions_is_normalised() -> Result<(), Box<dyn Error>> {
    let table = omop_cdm::meta::table("visit_occurrence").ok_or("no visit_occurrence table")?;
    let column = table
        .column("visit_type_concept_id")
        .ok_or("no visit_type_concept_id column")?;
    assert_eq!(
        "integer", column.cdm_datatype,
        "the one `Integer` of the definitions reached the generated metadata"
    );
    assert_eq!("i32", column.rust_type_name);
    assert!(column.required, "the column is required in the definitions");
    Ok(())
}

#[test]
fn a_foreign_key_names_a_table_and_a_column_of_the_model() -> Result<(), Box<dyn Error>> {
    let table = omop_cdm::meta::table("person").ok_or("no person table")?;
    let column = table
        .column("gender_concept_id")
        .ok_or("no gender_concept_id column")?;
    let (referenced, field) = column.foreign_key.ok_or("no foreign key")?;
    assert_eq!(("concept", "concept_id"), (referenced, field));
    let concept = omop_cdm::meta::table(referenced).ok_or("no concept table")?;
    assert!(
        concept.column(field).is_some(),
        "the foreign key names a column the concept table does not have"
    );
    Ok(())
}

#[test]
fn the_primary_key_of_a_table_is_marked_once() -> Result<(), Box<dyn Error>> {
    let table = omop_cdm::meta::table("measurement").ok_or("no measurement table")?;
    let keys: Vec<&str> = table
        .columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name)
        .collect();
    assert_eq!(vec!["measurement_id"], keys);
    Ok(())
}

/// The tables of `OMOP_CDMv5.4_Table_Level.csv`, in file order.
///
/// A record starts at the beginning of a line with the table name and its
/// schema; the prose fields that follow carry embedded newlines, and no
/// continuation line matches that shape.
fn defined_tables(csv: &str) -> Vec<&str> {
    csv.lines()
        .filter_map(|line| {
            let (table, rest) = line.split_once(',')?;
            let (schema, _) = rest.split_once(',')?;
            let known = matches!(schema, "CDM" | "VOCAB" | "RESULTS");
            (known
                && !table.is_empty()
                && table.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
            .then_some(table)
        })
        .collect()
}

/// The columns of every `CREATE TABLE` in OHDSI's rendered DDL, in file order.
fn ddl_columns(sql: &str) -> BTreeMap<&str, Vec<&str>> {
    let mut tables = BTreeMap::new();
    let mut current: Option<(&str, Vec<&str>)> = None;
    for line in sql.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("CREATE TABLE @cdmDatabaseSchema.") {
            current = Some((rest.trim_end_matches('(').trim(), Vec::new()));
            continue;
        }
        let Some((name, columns)) = current.as_mut() else {
            continue;
        };
        if let Some(column) = line.split_whitespace().next()
            && !column.starts_with(')')
        {
            columns.push(column.trim_matches('"'));
        }
        if line.ends_with(");") {
            tables.insert(*name, columns.clone());
            current = None;
        }
    }
    tables
}
