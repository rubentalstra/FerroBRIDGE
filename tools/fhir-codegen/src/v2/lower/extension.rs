// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The field and component extensions: optionality, length, conformance length and
//! standards status.

use std::collections::BTreeMap;

use crate::v2::definition::{Element, Extension, Scalar};
use crate::v2::lower::CONFORMANCE_LENGTH;
use crate::v2::lower::ConditionalCode;
use crate::v2::lower::ConformanceLength;
use crate::v2::lower::Defect;
use crate::v2::lower::Input;
use crate::v2::lower::LENGTH;
use crate::v2::lower::Length;
use crate::v2::lower::LowerError;
use crate::v2::lower::NumberError;
use crate::v2::lower::OPTIONALITY;
use crate::v2::lower::Optionality;
use crate::v2::lower::STANDARDS_STATUS;
use crate::v2::lower::StandardsStatus;
use crate::v2::lower::defect;
use crate::v2::lower::invalid;
use crate::v2::lower::number;

/// The v2 extensions of a field or a component.
pub(super) struct Extensions {
    pub(super) optionality: Optionality,
    pub(super) length: Option<Length>,
    pub(super) conformance_length: Option<ConformanceLength>,
    pub(super) standards_status: Option<StandardsStatus>,
}

/// Reads the extensions of a field (`standards_status` allowed) or a
/// component, each at most once and the optionality always.
pub(super) fn element_extensions(
    sourced: &Input<'_>,
    element: &Element,
    standards_status_allowed: bool,
) -> Result<Extensions, LowerError> {
    let mut optionality = None;
    let mut length = None;
    let mut conformance_length = None;
    let mut standards_status = None;
    for extension in &element.extension {
        let slot_taken = match extension.url.as_str() {
            OPTIONALITY => optionality
                .replace(lower_optionality(sourced, element, extension)?)
                .is_some(),
            LENGTH => length
                .replace(lower_length(sourced, element, extension)?)
                .is_some(),
            CONFORMANCE_LENGTH => conformance_length
                .replace(lower_conformance_length(sourced, element, extension)?)
                .is_some(),
            STANDARDS_STATUS if standards_status_allowed => standards_status
                .replace(lower_standards_status(sourced, element, extension)?)
                .is_some(),
            other => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("unknown extension {other}"),
                ));
            }
        };
        if slot_taken {
            return Err(invalid(
                sourced,
                &element.id,
                format!("extension {} given twice", extension.url),
            ));
        }
    }
    let optionality =
        optionality.ok_or_else(|| invalid(sourced, &element.id, "no optionality extension"))?;
    Ok(Extensions {
        optionality,
        length,
        conformance_length,
        standards_status,
    })
}

pub(super) fn value_code<'a>(
    sourced: &Input<'_>,
    element: &Element,
    extension: &'a Extension,
) -> Result<&'a str, LowerError> {
    if extension.value_integer.is_some()
        || extension.value_boolean.is_some()
        || !extension.extension.is_empty()
    {
        return Err(invalid(
            sourced,
            &element.id,
            format!("{} carries more than a code", extension.url),
        ));
    }
    extension.value_code.as_deref().ok_or_else(|| {
        invalid(
            sourced,
            &element.id,
            format!("{} carries no code", extension.url),
        )
    })
}

fn lower_optionality(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<Optionality, LowerError> {
    let code = value_code(sourced, element, extension)?;
    let simple = match code {
        "R" => Some(Optionality::R),
        "RE" => Some(Optionality::Re),
        "O" => Some(Optionality::O),
        "C" => Some(Optionality::C),
        "X" => Some(Optionality::X),
        "B" => Some(Optionality::B),
        "W" => Some(Optionality::W),
        "-" => {
            defect(sourced, &element.id, Defect::DashOptionality)?;
            Some(Optionality::Unstated)
        }
        _ => None,
    };
    if let Some(simple) = simple {
        return Ok(simple);
    }
    let conditional = code
        .strip_prefix("C(")
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|inner| inner.split_once('/'));
    let Some((first, second)) = conditional else {
        return Err(invalid(
            sourced,
            &element.id,
            format!("optionality {code:?}"),
        ));
    };
    let inner = |text: &str| match text {
        "R" => Ok(ConditionalCode::R),
        "RE" => Ok(ConditionalCode::Re),
        "O" => Ok(ConditionalCode::O),
        "X" => Ok(ConditionalCode::X),
        _ => Err(invalid(
            sourced,
            &element.id,
            format!("optionality {code:?}"),
        )),
    };
    Ok(Optionality::Conditional(inner(first)?, inner(second)?))
}

/// The value of one nested extension of a complex extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nested {
    /// A `valueInteger`, `None` where it is written `null`.
    Integer(Option<u64>),
    /// A `valueBoolean`.
    Boolean(bool),
}

/// The nested `valueInteger`s and `valueBoolean`s of a complex extension, by nested URL.
fn integers(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
    names: &[&str],
) -> Result<BTreeMap<String, Nested>, LowerError> {
    if extension.value_code.is_some()
        || extension.value_integer.is_some()
        || extension.value_boolean.is_some()
    {
        return Err(invalid(
            sourced,
            &element.id,
            format!("{} carries a value of its own", extension.url),
        ));
    }
    let mut values = BTreeMap::new();
    for nested in &extension.extension {
        if !names.contains(&nested.url.as_str())
            || nested.value_code.is_some()
            || !nested.extension.is_empty()
        {
            return Err(invalid(
                sourced,
                &element.id,
                format!("{} carries {:?}", extension.url, nested.url),
            ));
        }
        let value = match (&nested.value_integer, nested.value_boolean) {
            (None, Some(flag)) => Nested::Boolean(flag),
            (Some(_), Some(_)) => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("{} carries an integer and a boolean", nested.url),
                ));
            }
            (None, None) => {
                defect(sourced, &element.id, Defect::MissingInteger)?;
                Nested::Integer(None)
            }
            (Some(Scalar::Integer(number)), None) => Nested::Integer(Some(*number)),
            (Some(Scalar::Text(text)), None) => {
                defect(sourced, &element.id, Defect::TextInteger)?;
                Nested::Integer(Some(
                    text.parse::<u64>()
                        .map_err(NumberError::from)
                        .map_err(number(sourced, element, text))?,
                ))
            }
        };
        if values.insert(nested.url.clone(), value).is_some() {
            return Err(invalid(
                sourced,
                &element.id,
                format!("{} given twice", nested.url),
            ));
        }
    }
    Ok(values)
}

fn to_u32(sourced: &Input<'_>, element: &Element, value: u64) -> Result<u32, LowerError> {
    u32::try_from(value)
        .map_err(NumberError::from)
        .map_err(number(sourced, element, &value))
}

/// An integer nested in a complex extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NestedInteger {
    /// No nested extension of that name.
    Absent,
    /// A nested extension whose value is `null`.
    Null,
    /// A nested extension with a value.
    Value(u64),
}

/// The integer nested under `name`, refusing a boolean there.
fn nested_integer(
    sourced: &Input<'_>,
    element: &Element,
    values: &BTreeMap<String, Nested>,
    name: &str,
) -> Result<NestedInteger, LowerError> {
    match values.get(name) {
        None => Ok(NestedInteger::Absent),
        Some(Nested::Integer(None)) => Ok(NestedInteger::Null),
        Some(Nested::Integer(Some(value))) => Ok(NestedInteger::Value(*value)),
        Some(Nested::Boolean(_)) => Err(invalid(
            sourced,
            &element.id,
            format!("{name} is a boolean, not an integer"),
        )),
    }
}

fn lower_length(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<Length, LowerError> {
    let values = integers(sourced, element, extension, &["min", "max"])?;
    let NestedInteger::Value(min) = nested_integer(sourced, element, &values, "min")? else {
        return Err(invalid(sourced, &element.id, "a length without min"));
    };
    let max = match nested_integer(sourced, element, &values, "max")? {
        NestedInteger::Absent => {
            return Err(invalid(sourced, &element.id, "a length without max"));
        }
        NestedInteger::Null => None,
        NestedInteger::Value(max) => Some(max),
    };
    Ok(Length {
        min: to_u32(sourced, element, min)?,
        max: max.map(|max| to_u32(sourced, element, max)).transpose()?,
    })
}

fn lower_conformance_length(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<ConformanceLength, LowerError> {
    let values = integers(sourced, element, extension, &["length", "noTruncate"])?;
    let length = match nested_integer(sourced, element, &values, "length")? {
        NestedInteger::Value(length) => Some(to_u32(sourced, element, length)?),
        NestedInteger::Null => {
            return Err(invalid(
                sourced,
                &element.id,
                "a conformance length without a value",
            ));
        }
        NestedInteger::Absent => {
            defect(sourced, &element.id, Defect::ConformanceLengthWithoutLength)?;
            None
        }
    };
    // NOTE: no specification governs this: our own design; no definition of the
    // conformance-length extension is published, and fields write 1/0 where components write true/false.
    let no_truncate = match values.get("noTruncate") {
        Some(Nested::Integer(Some(0)) | Nested::Boolean(false)) => Some(false),
        Some(Nested::Integer(Some(1)) | Nested::Boolean(true)) => Some(true),
        Some(other) => {
            return Err(invalid(
                sourced,
                &element.id,
                format!("noTruncate {other:?} is neither 0 nor 1"),
            ));
        }
        None => {
            defect(
                sourced,
                &element.id,
                Defect::ConformanceLengthWithoutNoTruncate,
            )?;
            None
        }
    };
    Ok(ConformanceLength {
        length,
        no_truncate,
    })
}

fn lower_standards_status(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<StandardsStatus, LowerError> {
    match value_code(sourced, element, extension)? {
        "deprecated" => Ok(StandardsStatus::Deprecated),
        "withdrawn" => Ok(StandardsStatus::Withdrawn),
        other => Err(invalid(
            sourced,
            &element.id,
            format!("standards status {other:?}"),
        )),
    }
}
