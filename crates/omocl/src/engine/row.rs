// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The graph rows, built from the columns a record filled.
//!
//! A value takes the type its column's CDM datatype names
//! (`OMOP_CDMv5.4_Field_Level.csv`, carried by `omop_cdm::meta`), and the
//! graph's row builder checks the column, the type and every required column
//! as the row is built, so a value that does not fit (a string over its
//! `varchar(n)`, a fraction in an `integer`) refuses the record. The engine
//! fills `person_id` and `visit_occurrence_id` with references the writer
//! resolves, and the table's `*_type_concept_id` with the configured type
//! concept.

use std::collections::BTreeMap;

use omop_cdm::graph::GraphError;
use omop_cdm::graph::RecordKey;
use omop_cdm::graph::Reference;
use omop_cdm::graph::Row;
use omop_cdm::graph::Value;
use omop_cdm::graph::VisitKey;
use omop_cdm::meta::ColumnMeta;
use omop_cdm::value::CdmDate;
use omop_cdm::value::CdmDatetime;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// The suffix the CDM gives a table's provenance column.
const TYPE_CONCEPT_SUFFIX: &str = "_type_concept_id";

/// One column value, before its CDM type is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cell {
    /// A concept id or an integer.
    Int(i32),
    /// A number.
    Number(Decimal),
    /// Text.
    Text(String),
    /// A date.
    Date(CdmDate),
    /// A date and time.
    Datetime(CdmDatetime),
}

/// Why a row could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RowError {
    /// The graph refused the row.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// A value has no form in its column's CDM type.
    #[error("the value of `{column}` does not fit the column: {reason}")]
    Invalid {
        column: &'static str,
        reason: &'static str,
    },
}

/// The columns one record filled, by CDM column name.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cells {
    map: BTreeMap<&'static str, Cell>,
}

impl Cells {
    /// Sets one column.
    pub(crate) fn set(&mut self, column: &'static str, cell: Cell) {
        self.map.insert(column, cell);
    }
}

/// What the engine fills beside the mapped columns.
#[derive(Debug)]
pub(crate) struct Filled<'a> {
    pub(crate) person: &'a HierObjectId,
    pub(crate) visit: Option<&'a VisitKey>,
    pub(crate) type_concept: i32,
}

/// Returns a cell in the type its column takes.
fn typed(cell: &Cell, column: &ColumnMeta) -> Result<Value, RowError> {
    let invalid = |reason| RowError::Invalid {
        column: column.name,
        reason,
    };
    Ok(match (cell, column.cdm_datatype) {
        (&Cell::Int(value), "integer") => Value::Integer(value),
        (&Cell::Number(value), "integer") => {
            if !value.fract().is_zero() {
                return Err(invalid("an integer column takes no fraction"));
            }
            Value::Integer(
                value
                    .to_i32()
                    .ok_or_else(|| invalid("the number is outside the integer range"))?,
            )
        }
        (&Cell::Int(value), "float") => Value::Float(f64::from(value)),
        (&Cell::Number(value), "float") => Value::Float(
            value
                .to_f64()
                .ok_or_else(|| invalid("the number has no float form"))?,
        ),
        (Cell::Text(text), datatype) if datatype.starts_with("varchar(") => {
            Value::Text(text.clone())
        }
        (Cell::Date(date), "date") => Value::Date(date.clone()),
        (Cell::Datetime(datetime), "datetime") => Value::Datetime(datetime.clone()),
        _ => return Err(invalid("the value is not of the column's CDM type")),
    })
}

/// Builds the row of `table` from `cells`.
pub(crate) fn build(
    table: &str,
    key: RecordKey,
    cells: &Cells,
    filled: &Filled<'_>,
) -> Result<Row, RowError> {
    let meta = omop_cdm::graph::cdm_table(table)?;
    let mut builder = Row::builder(table, key)?;
    for (&name, cell) in &cells.map {
        let column = meta.column(name).ok_or_else(|| GraphError::UnknownColumn {
            table: meta.name,
            column: name.to_owned(),
        })?;
        builder = builder.value(name, typed(cell, column)?)?;
    }
    for column in meta.columns {
        if column.primary_key || cells.map.contains_key(column.name) {
            continue;
        }
        match column.foreign_key {
            Some(("person", _)) => {
                builder =
                    builder.reference(column.name, Reference::Person(filled.person.clone()))?;
            }
            Some(("visit_occurrence", _)) if column.name == "visit_occurrence_id" => {
                if let Some(visit) = filled.visit {
                    builder = builder.reference(column.name, Reference::Visit(visit.clone()))?;
                }
            }
            _ if column.name.ends_with(TYPE_CONCEPT_SUFFIX) => {
                builder = builder.value(column.name, Value::Integer(filled.type_concept))?;
            }
            _ => {}
        }
    }
    Ok(builder.build()?)
}
