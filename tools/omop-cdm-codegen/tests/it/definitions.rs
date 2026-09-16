// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading the two vendored definition files.

use std::error::Error;
use std::path::Path;

use omop_cdm_codegen::definitions::Definitions;

/// The vendored definitions directory.
pub(crate) const CSV_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/omop-cdm/inst/csv"
);

/// The vendored PostgreSQL DDL directory.
pub(crate) const DDL_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/omop-cdm/inst/ddl/5.4/postgresql"
);

#[test]
fn the_definitions_carry_39_tables_and_432_columns() -> Result<(), Box<dyn Error>> {
    let definitions = Definitions::load(Path::new(CSV_DIR))?;
    assert_eq!(39, definitions.tables.len());
    assert_eq!(432, definitions.fields.len());
    Ok(())
}

#[test]
fn every_field_record_names_a_defined_table() -> Result<(), Box<dyn Error>> {
    let definitions = Definitions::load(Path::new(CSV_DIR))?;
    for field in &definitions.fields {
        assert!(
            definitions
                .tables
                .iter()
                .any(|table| table.table == field.table),
            "the {} field names the undefined table {}",
            field.field,
            field.table
        );
    }
    Ok(())
}

#[test]
fn a_missing_definitions_directory_is_an_error() {
    assert!(Definitions::load(Path::new("/nonexistent/omop-cdm")).is_err());
}
