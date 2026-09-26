// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of a `Parameters` resource the four contracts share.

use fhir_types::codec::Json;
use fhir_types::codec::Path;
use fhir_types::operation::ParametersError;
use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::parameters::ParametersParameter;
use fhir_types::r4::parameters::ParametersParameterValue;
use fhir_types::r4::reference::Reference;

use crate::engine::context;
use crate::engine::context::CallContext;
use crate::operations::TO_FHIR;
use crate::operations::error::OperationError;

use crate::operations::contract::CONTEXT;

/// Reads a `Parameters` resource out of JSON text.
///
/// # Errors
///
/// Returns [`OperationError::Body`] when the text is not a `Parameters`
/// resource of the FHIR version in play.
pub fn parameters_from_json(
    operation: &'static str,
    text: &str,
) -> Result<Parameters, OperationError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|source| OperationError::Payload { source })?;
    let fhir_types::codec::Value::Object(object) = fhir_types::codec::Value::from_serde_json(value)
    else {
        return Err(OperationError::NotAnObject { what: "body" });
    };
    Parameters::from_json(&object, &mut Path::root("Parameters")).map_err(|source| {
        OperationError::Body {
            operation,
            expected: "Parameters",
            source: Box::new(source),
        }
    })
}

/// Returns the name of one parameter.
pub(super) fn named<'p>(
    operation: &'static str,
    rendered: &'static str,
    parameter: &'p ParametersParameter,
) -> Result<&'p str, OperationError> {
    parameter
        .name
        .value
        .as_deref()
        .ok_or(OperationError::Parameters {
            operation,
            source: ParametersError::Unnamed {
                operation: rendered,
            },
        })
}

/// Refuses a parameter that was already read.
pub(super) fn once(
    operation: &'static str,
    rendered: &'static str,
    name: &'static str,
    seen: bool,
) -> Result<(), OperationError> {
    if seen {
        return Err(OperationError::Parameters {
            operation,
            source: ParametersError::Repeated {
                operation: rendered,
                name,
            },
        });
    }
    Ok(())
}

/// Returns the `valueString` of one parameter.
pub(super) fn string<'p>(
    operation: &'static str,
    rendered: &'static str,
    name: &'static str,
    parameter: &'p ParametersParameter,
) -> Result<&'p str, OperationError> {
    let Some(ParametersParameterValue::String(ref carried)) = parameter.value else {
        return Err(OperationError::Parameters {
            operation,
            source: ParametersError::WrongType {
                operation: rendered,
                name,
                expected: "string",
            },
        });
    };
    carried.value.as_deref().ok_or(OperationError::Parameters {
        operation,
        source: ParametersError::MissingValue {
            operation: rendered,
            name,
        },
    })
}

/// Returns the `valueCode` of one parameter.
pub(super) fn code<'p>(
    operation: &'static str,
    rendered: &'static str,
    name: &'static str,
    parameter: &'p ParametersParameter,
) -> Result<&'p str, OperationError> {
    let carried = match parameter.value {
        // NOTE: the GET form of an operation carries no type marker at all
        // (<https://hl7.org/fhir/R4/operations.html>), so a `code` parameter
        // reads a `valueString` too.
        Some(ParametersParameterValue::Code(ref value)) => value.value.as_deref(),
        Some(ParametersParameterValue::String(ref value)) => value.value.as_deref(),
        _ => {
            return Err(OperationError::Parameters {
                operation,
                source: ParametersError::WrongType {
                    operation: rendered,
                    name,
                    expected: "code",
                },
            });
        }
    };
    carried.ok_or(OperationError::Parameters {
        operation,
        source: ParametersError::MissingValue {
            operation: rendered,
            name,
        },
    })
}

/// Returns the call context the `context` parameter's parts carry.
pub(super) fn call_context(parameter: &ParametersParameter) -> Result<CallContext, OperationError> {
    let mut carried = CallContext::new();
    let mut seen: Vec<&str> = Vec::new();
    for part in &parameter.part {
        let name = named(TO_FHIR, "$tofhir", part)?;
        if seen.contains(&name) {
            return Err(OperationError::Parameters {
                operation: TO_FHIR,
                source: ParametersError::Repeated {
                    operation: "$tofhir",
                    name: declared_part(name).unwrap_or(CONTEXT),
                },
            });
        }
        seen.push(name);
        match name {
            context::EHR_ID => {
                carried = carried.with_ehr_id(string(TO_FHIR, "$tofhir", "context.ehr_id", part)?);
            }
            context::PATIENT => {
                carried = carried.with_patient(reference(context::PATIENT, part)?);
            }
            context::WHO => {
                carried = carried.with_who(reference(context::WHO, part)?);
            }
            context::ON_BEHALF_OF => {
                carried = carried.with_on_behalf_of(reference(context::ON_BEHALF_OF, part)?);
            }
            other => {
                return Err(OperationError::Parameters {
                    operation: TO_FHIR,
                    source: ParametersError::Undeclared {
                        operation: "$tofhir",
                        name: format!("context.{other}"),
                    },
                });
            }
        }
    }
    Ok(carried)
}

/// Returns the declared part `name` spells, when it declares one.
fn declared_part(name: &str) -> Option<&'static str> {
    match name {
        context::EHR_ID => Some("context.ehr_id"),
        context::PATIENT => Some("context.patient"),
        context::WHO => Some("context.who"),
        context::ON_BEHALF_OF => Some("context.onBehalfOf"),
        _ => None,
    }
}

/// Returns the `valueReference` of one context part.
fn reference(name: &'static str, part: &ParametersParameter) -> Result<Reference, OperationError> {
    let Some(ParametersParameterValue::Reference(ref carried)) = part.value else {
        return Err(OperationError::Parameters {
            operation: TO_FHIR,
            source: ParametersError::WrongType {
                operation: "$tofhir",
                name: declared_part(name).unwrap_or(CONTEXT),
                expected: "Reference",
            },
        });
    };
    Ok(carried.as_ref().clone())
}

/// Returns one parameter carrying `value` under `name`.
pub(super) fn value_parameter(name: &str, value: ParametersParameterValue) -> ParametersParameter {
    ParametersParameter {
        name: fhir_types::r4::primitives::String::from(name),
        value: Some(value),
        ..ParametersParameter::default()
    }
}

/// Returns the `context` parameter `carried` writes as.
pub(super) fn context_parameter(carried: &CallContext) -> ParametersParameter {
    let mut part = Vec::new();
    if let Some(id) = carried.ehr_id() {
        part.push(value_parameter(
            context::EHR_ID,
            ParametersParameterValue::String(fhir_types::r4::primitives::String::from(id)),
        ));
    }
    for (name, value) in [
        (context::PATIENT, carried.patient()),
        (context::WHO, carried.who()),
        (context::ON_BEHALF_OF, carried.on_behalf_of()),
    ] {
        if let Some(value) = value {
            part.push(value_parameter(
                name,
                ParametersParameterValue::Reference(Box::new(value.clone())),
            ));
        }
    }
    ParametersParameter {
        name: fhir_types::r4::primitives::String::from(CONTEXT),
        part,
        ..ParametersParameter::default()
    }
}
