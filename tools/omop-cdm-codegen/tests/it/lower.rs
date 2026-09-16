// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The model the definitions lower to: the type mapping of `docs/architecture.md`
//! §10, the `Integer` normalisation, and the row type names.

use std::error::Error;
use std::path::Path;

use omop_cdm_codegen::definitions::Definitions;
use omop_cdm_codegen::lower::{Model, RustType, Schema, upper_camel_case};

use crate::definitions::CSV_DIR;

/// Lowers the vendored definitions.
fn model() -> Result<Model, Box<dyn Error>> {
    Ok(Model::lower(&Definitions::load(Path::new(CSV_DIR))?)?)
}

#[test]
fn every_cdm_datatype_maps_to_its_rust_type() -> Result<(), Box<dyn Error>> {
    let model = model()?;
    for table in &model.tables {
        for column in &table.columns {
            let expected = match column.cdm_datatype.as_str() {
                "integer" => RustType::Integer,
                "bigint" => RustType::BigInt,
                "float" => RustType::Float,
                "date" => RustType::Date,
                "datetime" => RustType::Datetime,
                "varchar(MAX)" => RustType::Text,
                other => {
                    assert!(
                        other.starts_with("varchar("),
                        "{}.{} carries the unmapped datatype {other}",
                        table.name,
                        column.name
                    );
                    column.rust_type
                }
            };
            assert_eq!(
                expected, column.rust_type,
                "{}.{} maps to the wrong Rust type",
                table.name, column.name
            );
        }
    }
    Ok(())
}

#[test]
fn the_capital_i_integer_is_normalised_where_the_definitions_carry_it() -> Result<(), Box<dyn Error>>
{
    let model = model()?;
    let table = model
        .tables
        .iter()
        .find(|table| table.name == "visit_occurrence")
        .ok_or("no visit_occurrence table")?;
    let column = table
        .columns
        .iter()
        .find(|column| column.name == "visit_type_concept_id")
        .ok_or("no visit_type_concept_id column")?;
    assert_eq!("integer", column.cdm_datatype);
    assert_eq!(RustType::Integer, column.rust_type);

    for table in &model.tables {
        for column in &table.columns {
            assert_ne!(
                "Integer", column.cdm_datatype,
                "{}.{} kept the definitions' capital-I spelling",
                table.name, column.name
            );
        }
    }
    Ok(())
}

#[test]
fn a_varchar_keeps_its_declared_length() -> Result<(), Box<dyn Error>> {
    let model = model()?;
    let table = model
        .tables
        .iter()
        .find(|table| table.name == "person")
        .ok_or("no person table")?;
    let column = table
        .columns
        .iter()
        .find(|column| column.name == "person_source_value")
        .ok_or("no person_source_value column")?;
    assert_eq!(RustType::Varchar(50), column.rust_type);
    assert_eq!(
        "crate::value::Varchar<50>",
        column.rust_type.name(),
        "the row struct spells the bounded string with its crate path"
    );
    Ok(())
}

#[test]
fn the_three_schemas_of_the_definitions_are_all_represented() -> Result<(), Box<dyn Error>> {
    let model = model()?;
    for schema in [Schema::Cdm, Schema::Vocab, Schema::Results] {
        assert!(
            model.tables.iter().any(|table| table.schema == schema),
            "no table lands in the {} schema",
            schema.as_str()
        );
    }
    Ok(())
}

#[test]
fn a_table_name_becomes_its_row_type_name() {
    assert_eq!("Person", upper_camel_case("person"));
    assert_eq!(
        "ConceptRelationship",
        upper_camel_case("concept_relationship")
    );
    assert_eq!("NoteNlp", upper_camel_case("note_nlp"));
}

#[test]
fn a_quoted_column_name_loses_its_sql_quoting() -> Result<(), Box<dyn Error>> {
    let model = model()?;
    let table = model
        .tables
        .iter()
        .find(|table| table.name == "note_nlp")
        .ok_or("no note_nlp table")?;
    assert!(
        table.columns.iter().any(|column| column.name == "offset"),
        "the definitions' quoted `\"offset\"` did not become the bare column name"
    );
    Ok(())
}
