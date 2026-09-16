// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading the two OMOP CDM v5.4 definition files.
//!
//! `OMOP_CDMv5.4_Table_Level.csv` carries one record per table and
//! `OMOP_CDMv5.4_Field_Level.csv` one record per column, both with quoted,
//! multi-line prose fields. The records are kept in file order, which is the
//! definition order the emitter writes out
//! (<https://ohdsi.github.io/CommonDataModel/cdm54.html>).

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The file name of the table definitions.
pub const TABLE_LEVEL: &str = "OMOP_CDMv5.4_Table_Level.csv";

/// The file name of the field definitions.
pub const FIELD_LEVEL: &str = "OMOP_CDMv5.4_Field_Level.csv";

/// One record of the table definitions, as the CSV spells it.
#[derive(Debug, Clone, Deserialize)]
pub struct TableRecord {
    /// The table name, lower case, for example `visit_occurrence`.
    #[serde(rename = "cdmTableName")]
    pub table: String,
    /// The CDM schema the table belongs to: `CDM`, `VOCAB` or `RESULTS`.
    #[serde(rename = "schema")]
    pub schema: String,
    /// Whether every CDM instance must carry the table.
    #[serde(rename = "isRequired")]
    pub required: String,
    /// The table's prose description.
    #[serde(rename = "tableDescription")]
    pub description: String,
    /// The conventions a reader of the table needs.
    #[serde(rename = "userGuidance")]
    pub user_guidance: String,
    /// The conventions an ETL writing the table needs.
    #[serde(rename = "etlConventions")]
    pub etl_conventions: String,
}

/// One record of the field definitions, as the CSV spells it.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldRecord {
    /// The table the column belongs to.
    #[serde(rename = "cdmTableName")]
    pub table: String,
    /// The column name, lower case, for example `person_id`.
    #[serde(rename = "cdmFieldName")]
    pub field: String,
    /// Whether the column is mandatory.
    #[serde(rename = "isRequired")]
    pub required: String,
    /// The CDM datatype, for example `integer` or `varchar(50)`.
    #[serde(rename = "cdmDatatype")]
    pub datatype: String,
    /// The conventions a reader of the column needs.
    #[serde(rename = "userGuidance")]
    pub user_guidance: String,
    /// The conventions an ETL writing the column needs.
    #[serde(rename = "etlConventions")]
    pub etl_conventions: String,
    /// Whether the column is the table's primary key.
    #[serde(rename = "isPrimaryKey")]
    pub primary_key: String,
    /// Whether the column references another table.
    #[serde(rename = "isForeignKey")]
    pub foreign_key: String,
    /// The referenced table, upper case, or `NA`.
    #[serde(rename = "fkTableName")]
    pub fk_table: String,
    /// The referenced column, upper case, or `NA`.
    #[serde(rename = "fkFieldName")]
    pub fk_field: String,
}

/// Both definition files, in file order.
#[derive(Debug, Clone)]
pub struct Definitions {
    /// The table records, in definition order.
    pub tables: Vec<TableRecord>,
    /// The field records, in definition order.
    pub fields: Vec<FieldRecord>,
}

/// A definition file could not be read.
///
/// The cause is the `csv` crate's error, which carries the I/O failure of a
/// missing file and the record position of a malformed one alike.
#[derive(Debug, thiserror::Error)]
#[error("cannot read the CDM definitions at {path}")]
pub struct LoadError {
    /// The path that was tried.
    pub path: PathBuf,
    /// The underlying error.
    #[source]
    pub source: csv::Error,
}

impl Definitions {
    /// Reads both definition files from `dir`.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when a file is missing or unreadable, or when a
    /// record does not match its header.
    pub fn load(dir: &Path) -> Result<Self, LoadError> {
        Ok(Self {
            tables: read_records(&dir.join(TABLE_LEVEL))?,
            fields: read_records(&dir.join(FIELD_LEVEL))?,
        })
    }
}

/// Reads every record of the CSV file at `path` into `T`, in file order.
fn read_records<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, LoadError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .map_err(|source| LoadError {
            path: path.to_path_buf(),
            source,
        })?;
    let mut records = Vec::new();
    for record in reader.deserialize() {
        records.push(record.map_err(|source| LoadError {
            path: path.to_path_buf(),
            source,
        })?);
    }
    Ok(records)
}
