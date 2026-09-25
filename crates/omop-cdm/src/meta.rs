// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The column metadata of the CDM v5.4 tables.
//!
//! Every row type in [`crate::generated`] has a `static` beside it holding one
//! [`ColumnMeta`] per column in definition order, and
//! [`crate::generated::TABLES`] indexes them all. The emitter writes both from
//! `OMOP_CDMv5.4_Field_Level.csv` and `OMOP_CDMv5.4_Table_Level.csv`
//! (<https://ohdsi.github.io/CommonDataModel/cdm54.html>), so a consumer that
//! reads the metadata reads the definitions.

/// One of the three schemas the CDM v5.4 definitions place a table in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CdmSchema {
    /// The clinical, health-system, health-economics and derived tables.
    Cdm,
    /// The standardized vocabulary tables.
    Vocab,
    /// The tables the cohort analyses write.
    Results,
}

impl CdmSchema {
    /// Returns the schema as the definitions spell it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cdm => "CDM",
            Self::Vocab => "VOCAB",
            Self::Results => "RESULTS",
        }
    }
}

/// One column of one CDM table.
///
/// The keys and the `isRequired` flag live here rather than in the row type,
/// which encodes only whether a column is optional: a primary key and a
/// foreign key are relationships between tables, and a row type that spelled
/// them would be a second, drifting copy of the definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnMeta {
    /// The column name, as the CDM spells it.
    pub name: &'static str,
    /// The CDM datatype, for example `integer` or `varchar(50)`.
    pub cdm_datatype: &'static str,
    /// The Rust type that carries the column's value.
    ///
    /// The row type's field is that type when [`ColumnMeta::required`] is
    /// true, and `Option` of it when it is false.
    pub rust_type_name: &'static str,
    /// Whether the column is mandatory.
    pub required: bool,
    /// Whether the column is the table's primary key.
    pub primary_key: bool,
    /// The table and the column a foreign key references, in that order,
    /// both spelled as [`TableMeta::name`] and [`ColumnMeta::name`] are.
    pub foreign_key: Option<(&'static str, &'static str)>,
    /// The vocabulary domain the definitions name for a concept column
    /// (`fkDomain`), for example `Measurement` for `measurement_concept_id`.
    ///
    /// A column whose definition writes `NA` carries `None`.
    pub fk_domain: Option<&'static str>,
    /// The schema of the table the column belongs to.
    pub cdm_schema: CdmSchema,
}

/// One table of the CDM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableMeta {
    /// The table name, as the CDM spells it.
    pub name: &'static str,
    /// The schema the table belongs to.
    pub cdm_schema: CdmSchema,
    /// The columns, in definition order.
    pub columns: &'static [ColumnMeta],
}

impl TableMeta {
    /// Returns the column of this table with `name`.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&'static ColumnMeta> {
        self.columns.iter().find(|column| column.name == name)
    }
}

/// Returns the table of the CDM with `name`.
///
/// # Examples
///
/// ```
/// let person = omop_cdm::meta::table("person").ok_or("no person table")?;
/// assert_eq!(omop_cdm::meta::CdmSchema::Cdm, person.cdm_schema);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn table(name: &str) -> Option<&'static TableMeta> {
    crate::generated::TABLES
        .iter()
        .find(|table| table.name == name)
}
