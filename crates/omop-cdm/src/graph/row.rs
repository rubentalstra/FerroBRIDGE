// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One CDM row: its cells, the values they hold, and the builder that checks
//! them against the generated column metadata.

use std::collections::BTreeMap;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;

use crate::graph::key::MappingName;
use crate::graph::key::RecordKey;
use crate::graph::key::VisitKey;
use crate::graph::varchar_limit;
use crate::meta::CdmSchema;
use crate::meta::ColumnMeta;
use crate::meta::TableMeta;
use crate::value::CdmDate;
use crate::value::CdmDatetime;

/// A column value that names another row, resolved to its id by the writer.
#[derive(Debug, Clone, PartialEq)]
pub enum Reference {
    /// The `PERSON` of an EHR, for a column that references `person`.
    Person(HierObjectId),
    /// A derived visit, for a column that references `visit_occurrence`.
    Visit(VisitKey),
    /// Another row, in the table the column's foreign key names.
    Row(RecordKey),
}

/// A column value in the form its CDM datatype takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// An `integer` column.
    Integer(i32),
    /// A `float` column; never a NaN or an infinity.
    Float(f64),
    /// A `varchar(n)` or `varchar(max)` column.
    Text(String),
    /// A `date` column.
    Date(CdmDate),
    /// A `datetime` column.
    Datetime(CdmDatetime),
}

impl Value {
    /// Returns the kind of value, as the refusals name it.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Integer(_) => "integer",
            Self::Float(_) => "float",
            Self::Text(_) => "text",
            Self::Date(_) => "date",
            Self::Datetime(_) => "datetime",
        }
    }
}

/// What one column of a row holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// A value.
    Value(Value),
    /// A reference the writer resolves to an id.
    Reference(Reference),
}

/// A row or a link the graph refuses.
///
/// The variants name tables, columns and keys, never a value, because a
/// value of a clinical row is patient data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GraphError {
    /// The table is not a CDM table.
    #[error("`{table}` is not a table of the CDM schema")]
    UnknownTable {
        /// The name that was given.
        table: String,
    },
    /// The table cannot hold a mapped row.
    #[error("`{table}` takes no mapped rows: {why}")]
    NotWritable {
        /// The table.
        table: &'static str,
        /// Why.
        why: &'static str,
    },
    /// The column is not a column of the table.
    #[error("`{table}` has no column `{column}`")]
    UnknownColumn {
        /// The table.
        table: &'static str,
        /// The name that was given.
        column: String,
    },
    /// The column is the primary key, which the writer assigns.
    #[error("`{table}.{column}` is the primary key, which the writer assigns")]
    PrimaryKey {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The column was set twice.
    #[error("`{table}.{column}` is set twice")]
    Duplicate {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The value is not of the column's CDM datatype.
    #[error("`{table}.{column}` is `{datatype}` and was given a {given} value")]
    Mismatch {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The CDM datatype.
        datatype: &'static str,
        /// The kind of value given.
        given: &'static str,
    },
    /// The text is longer than the column's `varchar(n)`.
    #[error("`{table}.{column}` holds {limit} characters and was given {length}")]
    TooLong {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The column's bound.
        limit: usize,
        /// The length given, in characters.
        length: usize,
    },
    /// The float is a NaN or an infinity, which `NUMERIC` cannot hold.
    #[error("`{table}.{column}` was given a float that is not finite")]
    NotFinite {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The column cannot carry this reference.
    #[error("`{table}.{column}` cannot carry a {reference} reference")]
    Reference {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The kind of reference given.
        reference: &'static str,
    },
    /// A required column is not set.
    #[error("`{table}.{column}` is required and is not set")]
    Missing {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The row's key names another composition than the graph's.
    #[error("the row {key} of `{table}` belongs to another composition than the graph")]
    ForeignRow {
        /// The table.
        table: &'static str,
        /// The row's key.
        key: Box<RecordKey>,
    },
    /// Two rows of one table carry the same key.
    #[error("two rows of `{table}` carry the key {key}")]
    DuplicateRow {
        /// The table.
        table: &'static str,
        /// The key.
        key: Box<RecordKey>,
    },
    /// A link names a row the graph does not hold.
    #[error("a link names the row {key} of `{table}`, which the graph does not hold")]
    DanglingLink {
        /// The table.
        table: &'static str,
        /// The key.
        key: Box<RecordKey>,
    },
}

/// Returns the CDM-schema table with `name`.
///
/// # Errors
///
/// Returns [`GraphError::UnknownTable`] when no table of the `CDM` schema has
/// that name.
pub fn cdm_table(name: &str) -> Result<&'static TableMeta, GraphError> {
    crate::meta::table(name)
        .filter(|table| table.cdm_schema == CdmSchema::Cdm)
        .ok_or_else(|| GraphError::UnknownTable {
            table: name.to_owned(),
        })
}

/// The tables the derivations rebuild whole, which no mapped row may enter.
pub const DERIVED_TABLES: [&str; 4] = [
    "observation_period",
    "condition_era",
    "drug_era",
    "dose_era",
];

/// Returns the single integer primary key of `table`, when it has one.
#[must_use]
pub fn primary_key(table: &TableMeta) -> Option<&'static ColumnMeta> {
    let mut keys = table.columns.iter().filter(|column| column.primary_key);
    match (keys.next(), keys.next()) {
        (Some(key), None) if key.cdm_datatype == "integer" => Some(key),
        _ => None,
    }
}

/// One CDM row, keyed by its source, with every column checked against the
/// table's metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    table: &'static TableMeta,
    key: RecordKey,
    cells: BTreeMap<&'static str, Cell>,
}

impl Row {
    /// Starts a row of `table` under `key`, produced by the mapping its
    /// discriminator names.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownTable`] when `table` is not a CDM-schema
    /// table and [`GraphError::NotWritable`] for `fact_relationship` (a
    /// [`Link`](crate::graph::link::Link) writes it), a derived table, or a table without a single
    /// integer primary key.
    pub fn builder(table: &str, key: RecordKey) -> Result<RowBuilder, GraphError> {
        let table = cdm_table(table)?;
        if table.name == "fact_relationship" {
            return Err(GraphError::NotWritable {
                table: table.name,
                why: "a Link writes it",
            });
        }
        if DERIVED_TABLES.contains(&table.name) {
            return Err(GraphError::NotWritable {
                table: table.name,
                why: "the derivations rebuild it whole",
            });
        }
        if primary_key(table).is_none() {
            return Err(GraphError::NotWritable {
                table: table.name,
                why: "it has no single integer primary key",
            });
        }
        Ok(RowBuilder {
            row: Self {
                table,
                key,
                cells: BTreeMap::new(),
            },
        })
    }

    /// Returns the table.
    #[must_use]
    pub fn table(&self) -> &'static TableMeta {
        self.table
    }

    /// Returns the natural key.
    #[must_use]
    pub fn key(&self) -> &RecordKey {
        &self.key
    }

    /// Returns the mapping that produced the row.
    #[must_use]
    pub fn mapping(&self) -> &MappingName {
        self.key.discriminator().mapping()
    }

    /// Returns the cells, by column name.
    #[must_use]
    pub fn cells(&self) -> &BTreeMap<&'static str, Cell> {
        &self.cells
    }

    /// Returns the cell of `column`, when it is set.
    #[must_use]
    pub fn cell(&self, column: &str) -> Option<&Cell> {
        self.cells.get(column)
    }
}

/// A [`Row`] being built; every setter checks the column first.
#[derive(Debug, Clone)]
pub struct RowBuilder {
    row: Row,
}

impl RowBuilder {
    /// Returns the column `name` of the row's table, refusing the primary key
    /// and a column set before.
    fn column(&self, name: &str) -> Result<&'static ColumnMeta, GraphError> {
        let table = self.row.table;
        let column = table
            .column(name)
            .ok_or_else(|| GraphError::UnknownColumn {
                table: table.name,
                column: name.to_owned(),
            })?;
        if column.primary_key {
            return Err(GraphError::PrimaryKey {
                table: table.name,
                column: column.name,
            });
        }
        if self.row.cells.contains_key(column.name) {
            return Err(GraphError::Duplicate {
                table: table.name,
                column: column.name,
            });
        }
        Ok(column)
    }

    /// Sets `column` to `value`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownColumn`], [`GraphError::PrimaryKey`] or
    /// [`GraphError::Duplicate`] for a column that cannot be set,
    /// [`GraphError::Mismatch`] when the value is not of the column's
    /// datatype, [`GraphError::TooLong`] for text beyond a `varchar(n)`, and
    /// [`GraphError::NotFinite`] for a NaN or an infinity.
    pub fn value(mut self, column: &str, value: Value) -> Result<Self, GraphError> {
        let meta = self.column(column)?;
        let table = self.row.table.name;
        let mismatch = || GraphError::Mismatch {
            table,
            column: meta.name,
            datatype: meta.cdm_datatype,
            given: value.kind(),
        };
        match (&value, meta.cdm_datatype) {
            (Value::Integer(_), "integer")
            | (Value::Date(_), "date")
            | (Value::Datetime(_), "datetime") => {}
            (Value::Float(number), "float") => {
                if !number.is_finite() {
                    return Err(GraphError::NotFinite {
                        table,
                        column: meta.name,
                    });
                }
            }
            (Value::Text(text), datatype) if datatype.starts_with("varchar(") => {
                if let Some(limit) = varchar_limit(datatype) {
                    let length = text.chars().count();
                    if length > limit {
                        return Err(GraphError::TooLong {
                            table,
                            column: meta.name,
                            limit,
                            length,
                        });
                    }
                } else if datatype != "varchar(MAX)" {
                    return Err(mismatch());
                }
            }
            _ => return Err(mismatch()),
        }
        self.row.cells.insert(meta.name, Cell::Value(value));
        Ok(self)
    }

    /// Sets `column` to `value` when there is one, and leaves it unset
    /// otherwise.
    ///
    /// # Errors
    ///
    /// Returns what [`RowBuilder::value`] returns.
    pub fn optional(self, column: &str, value: Option<Value>) -> Result<Self, GraphError> {
        match value {
            Some(value) => self.value(column, value),
            None => Ok(self),
        }
    }

    /// Sets `column` to a reference the writer resolves.
    ///
    /// A [`Reference::Person`] fits a column whose foreign key is
    /// `person.person_id`, a [`Reference::Visit`] one whose foreign key is
    /// `visit_occurrence.visit_occurrence_id`, and a [`Reference::Row`] one
    /// whose foreign key names any other CDM table with an integer key.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownColumn`], [`GraphError::PrimaryKey`] or
    /// [`GraphError::Duplicate`] for a column that cannot be set, and
    /// [`GraphError::Reference`] when the column's foreign key does not fit
    /// the reference.
    pub fn reference(mut self, column: &str, reference: Reference) -> Result<Self, GraphError> {
        let meta = self.column(column)?;
        let target = meta.foreign_key.map(|(table, _)| table);
        let fits = match (&reference, target) {
            (Reference::Person(_), Some("person"))
            | (Reference::Visit(_), Some("visit_occurrence")) => true,
            (Reference::Row(_), Some(table)) => {
                !matches!(table, "person" | "visit_occurrence")
                    && crate::meta::table(table)
                        .filter(|target| target.cdm_schema == CdmSchema::Cdm)
                        .and_then(primary_key)
                        .is_some()
            }
            _ => false,
        };
        if !fits {
            return Err(GraphError::Reference {
                table: self.row.table.name,
                column: meta.name,
                reference: match reference {
                    Reference::Person(_) => "person",
                    Reference::Visit(_) => "visit",
                    Reference::Row(_) => "row",
                },
            });
        }
        self.row.cells.insert(meta.name, Cell::Reference(reference));
        Ok(self)
    }

    /// Finishes the row.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Missing`] naming the first required column, in
    /// definition order, that is not set. A required date is never filled in.
    pub fn build(self) -> Result<Row, GraphError> {
        let table = self.row.table;
        for column in table.columns {
            if column.required && !column.primary_key && !self.row.cells.contains_key(column.name) {
                return Err(GraphError::Missing {
                    table: table.name,
                    column: column.name,
                });
            }
        }
        Ok(self.row)
    }
}
