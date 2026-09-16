// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Rust model the emitter renders: one table per module, one column per
//! field, each CDM datatype mapped to the Rust type that carries it.
//!
//! The mapping is fixed by `docs/architecture.md` §10 and the CDM's own
//! datatypes (<https://ohdsi.github.io/CommonDataModel/cdm54.html>).

use std::collections::{BTreeMap, BTreeSet};

use crate::definitions::{Definitions, FieldRecord, TableRecord};

/// Every Rust keyword, strict and reserved, that a column name would collide
/// with (<https://doc.rust-lang.org/reference/keywords.html>).
const RUST_KEYWORDS: [&str; 53] = [
    "abstract",
    "as",
    "async",
    "await",
    "become",
    "box",
    "break",
    "const",
    "continue",
    "crate",
    "do",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "macro",
    "match",
    "mod",
    "move",
    "mut",
    "override",
    "priv",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "unsafe",
    "unsized",
    "use",
    "virtual",
    "where",
    "while",
    "yield",
    "union",
    "macro_rules",
];

/// The Rust type a CDM datatype maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustType {
    /// `integer`, the CDM's 32-bit integer and every surrogate key.
    Integer,
    /// `bigint`, a 64-bit integer.
    BigInt,
    /// `float`, a double-precision number.
    Float,
    /// `varchar(n)`, a string of at most `n` characters.
    Varchar(usize),
    /// `varchar(MAX)`, a string with no declared bound.
    Text,
    /// `date`, kept lexically.
    Date,
    /// `datetime`, kept lexically.
    Datetime,
}

impl RustType {
    /// Returns the Rust type's name as the emitter spells it in a row struct.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Integer => String::from("i32"),
            Self::BigInt => String::from("i64"),
            Self::Float => String::from("f64"),
            Self::Varchar(limit) => format!("crate::value::Varchar<{limit}>"),
            Self::Text => String::from("String"),
            Self::Date => String::from("crate::value::CdmDate"),
            Self::Datetime => String::from("crate::value::CdmDatetime"),
        }
    }
}

/// One column of one CDM table.
#[derive(Debug, Clone)]
pub struct Column {
    /// The column name, for example `person_id`.
    pub name: String,
    /// The CDM datatype, normalised to the spelling the definitions use
    /// everywhere else.
    pub cdm_datatype: String,
    /// The Rust type that carries the column's value.
    pub rust_type: RustType,
    /// Whether the column is mandatory.
    pub required: bool,
    /// Whether the column is the table's primary key.
    pub primary_key: bool,
    /// The table and column a foreign key references, both lower case.
    pub foreign_key: Option<(String, String)>,
    /// The definitions' `userGuidance` text, when there is any.
    pub user_guidance: Option<String>,
    /// The definitions' `etlConventions` text, when there is any.
    pub etl_conventions: Option<String>,
}

/// One CDM table: its module, its row type, and its columns in definition
/// order.
#[derive(Debug, Clone)]
pub struct Table {
    /// The table name, for example `visit_occurrence`.
    pub name: String,
    /// The row type's name, for example `VisitOccurrence`.
    pub type_name: String,
    /// The CDM schema the table belongs to.
    pub schema: Schema,
    /// The definitions' `tableDescription` text, when there is any.
    pub description: Option<String>,
    /// The definitions' `userGuidance` text, when there is any.
    pub user_guidance: Option<String>,
    /// The definitions' `etlConventions` text, when there is any.
    pub etl_conventions: Option<String>,
    /// The columns, in definition order.
    pub columns: Vec<Column>,
}

impl Table {
    /// Returns the name of the table's column-metadata static.
    #[must_use]
    pub fn columns_static(&self) -> String {
        format!("{}_COLUMNS", self.name.to_uppercase())
    }
}

/// The CDM schema a table belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    /// The clinical and health-system tables.
    Cdm,
    /// The standardized vocabulary tables.
    Vocab,
    /// The results tables the cohort analyses write.
    Results,
}

impl Schema {
    /// Returns the variant of `omop_cdm::meta::CdmSchema` this schema is.
    #[must_use]
    pub fn variant(self) -> &'static str {
        match self {
            Self::Cdm => "Cdm",
            Self::Vocab => "Vocab",
            Self::Results => "Results",
        }
    }

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

/// The whole generated model: every table of the definitions, in definition
/// order.
#[derive(Debug, Clone)]
pub struct Model {
    /// The tables, in definition order.
    pub tables: Vec<Table>,
}

/// A definition record the emitter cannot map.
#[derive(Debug, thiserror::Error)]
pub enum LowerError {
    /// The datatype is not one of the CDM's.
    #[error("the {table}.{column} column has the unknown CDM datatype `{datatype}`")]
    Datatype {
        /// The table.
        table: String,
        /// The column.
        column: String,
        /// The datatype the definitions carry.
        datatype: String,
    },
    /// A `varchar(n)` length did not parse.
    #[error("the {table}.{column} column has an unreadable varchar length in `{datatype}`")]
    VarcharLength {
        /// The table.
        table: String,
        /// The column.
        column: String,
        /// The datatype the definitions carry.
        datatype: String,
        /// The underlying error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// A boolean column of the definitions is neither `TRUE` nor `FALSE`.
    #[error("the {table}.{column} column has `{value}` in its {flag} flag, not TRUE or FALSE")]
    Flag {
        /// The table.
        table: String,
        /// The column, or the table name again for a table-level flag.
        column: String,
        /// The flag that did not parse.
        flag: &'static str,
        /// The value the definitions carry.
        value: String,
    },
    /// The schema is not one of the three the CDM defines.
    #[error("the {table} table names the unknown CDM schema `{schema}`")]
    Schema {
        /// The table.
        table: String,
        /// The schema the definitions carry.
        schema: String,
    },
    /// A foreign key names no table or no column.
    #[error("the {table}.{column} foreign key names no referenced table and column")]
    ForeignKey {
        /// The table.
        table: String,
        /// The column.
        column: String,
    },
    /// A table of the table definitions has no field definitions.
    #[error("the {table} table has no columns in the field definitions")]
    NoColumns {
        /// The table.
        table: String,
    },
    /// A column name is a Rust keyword, so it cannot be a field name.
    #[error("the {table}.{column} column is named after a Rust keyword")]
    ReservedName {
        /// The table.
        table: String,
        /// The column.
        column: String,
    },
    /// A field definition names a table the table definitions do not carry.
    #[error("the field definitions name the {table} table, which the table definitions do not")]
    UnknownTable {
        /// The table.
        table: String,
    },
}

impl Model {
    /// Lowers both definition files into the model the emitter renders.
    ///
    /// # Errors
    ///
    /// Returns [`LowerError`] when a record carries a datatype, flag, schema
    /// or foreign key the emitter cannot map, and when a defined table has no
    /// columns.
    pub fn lower(definitions: &Definitions) -> Result<Self, LowerError> {
        let mut by_table: BTreeMap<&str, Vec<&FieldRecord>> = BTreeMap::new();
        for field in &definitions.fields {
            by_table.entry(&field.table).or_default().push(field);
        }
        let defined: BTreeSet<&str> = definitions
            .tables
            .iter()
            .map(|record| record.table.as_str())
            .collect();
        for table in by_table.keys() {
            if !defined.contains(table) {
                return Err(LowerError::UnknownTable {
                    table: (*table).to_owned(),
                });
            }
        }
        let mut tables = Vec::with_capacity(definitions.tables.len());
        for record in &definitions.tables {
            let fields =
                by_table
                    .get(record.table.as_str())
                    .ok_or_else(|| LowerError::NoColumns {
                        table: record.table.clone(),
                    })?;
            tables.push(lower_table(record, fields)?);
        }
        Ok(Self { tables })
    }
}

/// Lowers one table record and its field records.
fn lower_table(record: &TableRecord, fields: &[&FieldRecord]) -> Result<Table, LowerError> {
    let schema = match record.schema.as_str() {
        "CDM" => Schema::Cdm,
        "VOCAB" => Schema::Vocab,
        "RESULTS" => Schema::Results,
        other => {
            return Err(LowerError::Schema {
                table: record.table.clone(),
                schema: other.to_owned(),
            });
        }
    };
    let mut columns = Vec::with_capacity(fields.len());
    for field in fields {
        columns.push(lower_column(field)?);
    }
    Ok(Table {
        name: record.table.clone(),
        type_name: upper_camel_case(&record.table),
        schema,
        description: prose(&record.description),
        user_guidance: prose(&record.user_guidance),
        etl_conventions: prose(&record.etl_conventions),
        columns,
    })
}

/// Lowers one field record.
fn lower_column(field: &FieldRecord) -> Result<Column, LowerError> {
    // NOTE: the definitions quote `"offset"` because OFFSET is a reserved word
    // in SQL, and OHDSI's DDL quotes it too; the column is the bare name.
    let name = field.field.trim().trim_matches('"').to_owned();
    if RUST_KEYWORDS.contains(&name.as_str()) {
        return Err(LowerError::ReservedName {
            table: field.table.clone(),
            column: name,
        });
    }
    // NOTE: `visit_occurrence.visit_type_concept_id` is the one field of the
    // v5.4.3 definitions spelled `Integer`; normalising it here keeps the
    // emitted model uniform, and the typo is reported upstream on #103.
    let datatype = if field.datatype == "Integer" {
        String::from("integer")
    } else {
        field.datatype.clone()
    };
    let rust_type = rust_type(field, &datatype)?;
    let foreign_key = if flag(field, "isForeignKey", &field.foreign_key)? {
        let table = named(&field.fk_table);
        let column = named(&field.fk_field);
        match (table, column) {
            (Some(table), Some(column)) => Some((table.to_lowercase(), column.to_lowercase())),
            _ => {
                return Err(LowerError::ForeignKey {
                    table: field.table.clone(),
                    column: field.field.clone(),
                });
            }
        }
    } else {
        None
    };
    Ok(Column {
        name,
        cdm_datatype: datatype,
        rust_type,
        required: flag(field, "isRequired", &field.required)?,
        primary_key: flag(field, "isPrimaryKey", &field.primary_key)?,
        foreign_key,
        user_guidance: prose(&field.user_guidance),
        etl_conventions: prose(&field.etl_conventions),
    })
}

/// Maps one normalised CDM datatype onto its Rust type.
fn rust_type(field: &FieldRecord, datatype: &str) -> Result<RustType, LowerError> {
    match datatype {
        "integer" => Ok(RustType::Integer),
        "bigint" => Ok(RustType::BigInt),
        "float" => Ok(RustType::Float),
        "date" => Ok(RustType::Date),
        "datetime" => Ok(RustType::Datetime),
        "varchar(MAX)" => Ok(RustType::Text),
        other => {
            let length = other
                .strip_prefix("varchar(")
                .and_then(|rest| rest.strip_suffix(')'))
                .ok_or_else(|| LowerError::Datatype {
                    table: field.table.clone(),
                    column: field.field.clone(),
                    datatype: other.to_owned(),
                })?;
            let length = length
                .parse::<usize>()
                .map_err(|source| LowerError::VarcharLength {
                    table: field.table.clone(),
                    column: field.field.clone(),
                    datatype: other.to_owned(),
                    source,
                })?;
            Ok(RustType::Varchar(length))
        }
    }
}

/// Reads one `TRUE`/`FALSE` flag of the field definitions.
fn flag(field: &FieldRecord, name: &'static str, value: &str) -> Result<bool, LowerError> {
    match value.trim() {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        other => Err(LowerError::Flag {
            table: field.table.clone(),
            column: field.field.clone(),
            flag: name,
            value: other.to_owned(),
        }),
    }
}

/// Returns the trimmed prose of a definition cell, or `None` when the cell is
/// empty or the definitions' `NA`.
fn prose(value: &str) -> Option<String> {
    named(value).map(str::to_owned)
}

/// Returns the trimmed cell, or `None` when it is empty or the `NA` the
/// definitions write for an absent value.
fn named(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "NA" {
        None
    } else {
        Some(trimmed)
    }
}

/// Converts a snake-case CDM name into the row type's `UpperCamelCase` name.
#[must_use]
pub fn upper_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for word in name.split('_') {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.extend(chars.flat_map(char::to_lowercase));
        }
    }
    out
}
