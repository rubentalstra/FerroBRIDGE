// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The values an alternative produces, decoded from canonical JSON into the
//! generated openEHR RM types.
//!
//! A node is decoded by its `_type` into the `openehr-rm` class it names
//! (`DataValue` for a data value, `Element` for an `ELEMENT`,
//! <https://docs.rs/openehr-rm/0.0.69/openehr_rm/v1_2/data_types/basic/data_value/enum.DataValue.html>),
//! and every part the engine takes is read from the decoded struct's fields.
//! An `ELEMENT` with no value (a `null_flavour`) reads as absent, so the next
//! alternative is tried. A structure reads the one mandatory `DV_DATE_TIME`
//! attribute `openehr_rm::v1_2::model` gives its class (`EVENT.time`,
//! `HISTORY.origin`, `EVENT_CONTEXT.start_time`, `ACTION.time`). OMOCL states
//! no reading of a date from a structure, so taking that attribute is our own
//! design.

use core::str::FromStr;

use omop_cdm::value::CdmDate;
use omop_cdm::value::CdmDatetime;
use openehr_base::v1_2::foundation_types::time::iso8601_date::Iso8601Date;
use openehr_base::v1_2::foundation_types::time::iso8601_date_time::Iso8601DateTime;
use openehr_rm::v1_2::data_structures::representation::element::Element;
use openehr_rm::v1_2::data_types::basic::data_value::DataValue;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_quantity::DvQuantity;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use openehr_rm::v1_2::model;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::Predicate;
use openehr_rm::v1_2::paths::select_children;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::model::ast::ConceptId;

/// A decimal with the text it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Number {
    pub(crate) value: Decimal,
    pub(crate) text: String,
}

/// A date, with its time when the source carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Moment {
    pub(crate) date: CdmDate,
    pub(crate) datetime: Option<CdmDatetime>,
    pub(crate) written: String,
}

/// One value an alternative produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Datum {
    /// A `DV_CODED_TEXT`: its defining code and its text.
    Coded { code: CodePhrase, text: String },
    /// A `DV_TEXT`.
    Text(String),
    /// A `DV_QUANTITY`, with its magnitude as an exact decimal.
    Quantity {
        quantity: Box<DvQuantity>,
        magnitude: Number,
    },
    /// A `DV_COUNT`, a `DV_ORDINAL` or `DV_SCALE` value, or a product.
    Number(Number),
    /// A `DV_DATE_TIME` or `DV_DATE`, or the date of a structure.
    Moment(Moment),
    /// A `DV_BOOLEAN`.
    Flag(bool),
    /// A `DV_IDENTIFIER`'s `id`.
    Identifier(String),
    /// A literal `code`.
    Literal(ConceptId),
    /// A `conceptMap` hit: the concept and the at-code it was read from.
    Mapped { concept: ConceptId, code: String },
}

/// Why a value could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DatumError {
    /// The node is of an openEHR class the engine reads no value from.
    #[error("the node is a {rm_type}, which carries no value a CDM column takes")]
    Unsupported { rm_type: String },
    /// The node does not decode as the class it names.
    #[error("the {rm_type} value does not decode: {reason}")]
    Malformed { rm_type: String, reason: String },
}

/// Returns the class a canonical JSON node names.
fn class_of(node: &Value) -> Result<&str, DatumError> {
    node.get("_type")
        .and_then(Value::as_str)
        .ok_or_else(|| DatumError::Unsupported {
            rm_type: "a node with no `_type`".to_owned(),
        })
}

/// Decodes a node as `T`.
fn decode<T: serde::de::DeserializeOwned>(node: &Value, rm_type: &str) -> Result<T, DatumError> {
    serde_json::from_value(node.clone()).map_err(|error| DatumError::Malformed {
        rm_type: rm_type.to_owned(),
        reason: error.to_string(),
    })
}

/// Reads the value a node carries, `None` when it carries none.
pub(crate) fn read(node: &Value) -> Result<Option<Datum>, DatumError> {
    let class = class_of(node)?;
    if model::is_a(class, "DATA_VALUE") {
        return value(decode::<DataValue>(node, class)?, class).map(Some);
    }
    if model::is_a(class, "ELEMENT") {
        let element = decode::<Element>(node, class)?;
        return element
            .value
            .map(|carried| value(carried, class))
            .transpose();
    }
    let dates: Vec<&str> = model::attributes(class)
        .filter(|attribute| attribute.is_mandatory && attribute.declared_type == "DV_DATE_TIME")
        .map(|attribute| attribute.name)
        .collect();
    let [attribute] = dates.as_slice() else {
        return Err(DatumError::Unsupported {
            rm_type: class.to_owned(),
        });
    };
    let segment = PathSegment {
        attribute: (*attribute).to_owned(),
        predicate: Predicate::default(),
        descendant: false,
    };
    match select_children(node, &segment).as_slice() {
        [time] => {
            let time = decode::<DvDateTime>(time, "DV_DATE_TIME")?;
            Ok(Some(Datum::Moment(moment(&time.value)?)))
        }
        _ => Ok(None),
    }
}

/// Reads one decoded data value.
fn value(value: DataValue, class: &str) -> Result<Datum, DatumError> {
    Ok(match value {
        DataValue::DvText(DvText::DvCodedText(coded)) => Datum::Coded {
            code: coded.defining_code,
            text: coded.value,
        },
        DataValue::DvText(DvText::DvText(text)) => Datum::Text(text.value),
        DataValue::DvQuantity(quantity) => Datum::Quantity {
            magnitude: number(quantity.magnitude, "DV_QUANTITY")?,
            quantity: Box::new(quantity),
        },
        DataValue::DvCount(count) => Datum::Number(Number {
            value: Decimal::from(count.magnitude),
            text: count.magnitude.to_string(),
        }),
        DataValue::DvOrdinal(ordinal) => Datum::Number(Number {
            value: Decimal::from(ordinal.value),
            text: ordinal.value.to_string(),
        }),
        DataValue::DvScale(scale) => Datum::Number(number(scale.value, "DV_SCALE")?),
        DataValue::DvDateTime(date_time) => Datum::Moment(moment(&date_time.value)?),
        DataValue::DvDate(date) => Datum::Moment(date_only(&date.value)?),
        DataValue::DvBoolean(flag) => Datum::Flag(flag.value),
        DataValue::DvIdentifier(identifier) => Datum::Identifier(identifier.id),
        _ => {
            return Err(DatumError::Unsupported {
                rm_type: class.to_owned(),
            });
        }
    })
}

/// Returns a magnitude as an exact decimal.
///
/// The decimal is parsed from the shortest text that round-trips the float
/// (<https://doc.rust-lang.org/std/primitive.f64.html#impl-Display-for-f64>),
/// so a factor carries the digits the source wrote into the product.
pub(crate) fn number(magnitude: f64, rm_type: &str) -> Result<Number, DatumError> {
    let text = magnitude.to_string();
    let value = Decimal::from_str_exact(&text).map_err(|error| DatumError::Malformed {
        rm_type: rm_type.to_owned(),
        reason: error.to_string(),
    })?;
    Ok(Number { value, text })
}

/// Returns the refusal of a date that is not complete.
fn incomplete(rm_type: &str) -> DatumError {
    DatumError::Malformed {
        rm_type: rm_type.to_owned(),
        reason: "not a complete ISO 8601 date".to_owned(),
    }
}

/// Returns the CDM date of a year, month and day.
fn cdm_date(
    parts: (Option<u32>, Option<u32>, Option<u32>),
    rm_type: &str,
) -> Result<CdmDate, DatumError> {
    let (Some(year), Some(month), Some(day)) = parts else {
        return Err(incomplete(rm_type));
    };
    CdmDate::new(format!("{year:04}-{month:02}-{day:02}")).map_err(|_refused| incomplete(rm_type))
}

/// Reads an ISO 8601 date and time into the CDM's two columns.
///
/// The components come from `openehr-base`'s `Iso8601DateTime`. The CDM
/// `datetime` is a timestamp without a time zone, so the wall-clock time the
/// source writes is kept and its offset is not applied, and a missing minute
/// or second is written as `00`. No specification governs this reading: our
/// own design. A value with no time is read as a date, and a partial date has
/// no CDM date and is refused.
pub(crate) fn moment(written: &str) -> Result<Moment, DatumError> {
    let parsed = Iso8601DateTime {
        value: written.to_owned(),
    };
    let Some(hour) = parsed.hour() else {
        return date_only(written);
    };
    // NOTE: no specification governs this: our own design; the CDM `datetime`
    // columns carry no zone, so the wall-clock time is kept and the offset dropped.
    let date = cdm_date(
        (parsed.year(), parsed.month(), parsed.day()),
        "DV_DATE_TIME",
    )?;
    let minute = parsed.minute().unwrap_or(0);
    let second = parsed.second().unwrap_or(0);
    let datetime = CdmDatetime::from_str(&format!("{date}T{hour:02}:{minute:02}:{second:02}"))
        .map_err(|_refused| incomplete("DV_DATE_TIME"))?;
    Ok(Moment {
        date,
        datetime: Some(datetime),
        written: written.to_owned(),
    })
}

/// Reads an ISO 8601 date with no time.
fn date_only(written: &str) -> Result<Moment, DatumError> {
    let parsed = Iso8601Date {
        value: written.to_owned(),
    };
    Ok(Moment {
        date: cdm_date((parsed.year(), parsed.month(), parsed.day()), "DV_DATE")?,
        datetime: None,
        written: written.to_owned(),
    })
}
