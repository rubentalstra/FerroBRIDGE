// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed request and response of each operation.
//!
//! The FHIR R4 operations framework carries an invocation in a `Parameters`
//! resource and answers with one too, except where a single `out` parameter
//! named `return` carries a resource, which is returned directly
//! (<https://hl7.org/fhir/R4/operations.html>). `ToFhir.fsh` declares exactly
//! that shape, so `$tofhir` answers a `Bundle`; `ToOpenEhr.fsh` declares two
//! `out` parameters, so `$toopenehr` answers a `Parameters`.
//!
//! Reading is strict in both directions: a parameter the operation does not
//! declare, one given twice, one carrying the wrong value type and a flat
//! composition with no `templateId` are each a typed refusal.

pub mod parameters;

use fhir_types::operation::ParametersError;
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::operation_outcome::OperationOutcome;
use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::parameters::ParametersParameter;
use fhir_types::r4::parameters::ParametersParameterValue;
use fhir_types::r4::resource::Resource;

use crate::engine::context::CallContext;
use crate::operations::TO_FHIR;
use crate::operations::TO_OPENEHR;
use crate::operations::error::OperationError;
use crate::resolve::program::binding::TemplateId;

use crate::operations::contract::parameters::call_context;
use crate::operations::contract::parameters::code;
use crate::operations::contract::parameters::context_parameter;
use crate::operations::contract::parameters::named;
use crate::operations::contract::parameters::once;
use crate::operations::contract::parameters::string;
use crate::operations::contract::parameters::value_parameter;

/// The `composition` parameter name.
const COMPOSITION: &str = "composition";

/// The `templateId` parameter name.
const TEMPLATE_ID: &str = "templateId";

/// The `context` parameter name.
const CONTEXT: &str = "context";

/// The `bundle` parameter name.
const BUNDLE: &str = "bundle";

/// The `format` parameter name.
const FORMAT: &str = "format";

/// The `outcome` parameter name.
const OUTCOME: &str = "outcome";

/// The serialization a composition is carried in.
///
/// "openEHR Compositions are exchanged in two interchangeable JSON
/// serializations, and an engine MUST accept both wherever a Composition is
/// supplied" (`engine/rest-api.adoc` §Composition formats).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionPayload {
    /// The openEHR Reference Model structure, with `_type` on every node.
    Canonical(serde_json::Map<String, serde_json::Value>),
    /// The single-level simSDT object whose keys are template paths.
    Flat(serde_json::Map<String, serde_json::Value>),
}

impl CompositionPayload {
    /// Reads `text` as a composition in whichever serialization it carries.
    ///
    /// A canonical composition is the Reference Model structure and carries
    /// `_type`; a flat one is a single-level object of template paths and
    /// carries none. The draft names no discriminator, so reading `_type` is
    /// FerroBRIDGE's own design (recorded on FerroBRIDGE issue #192).
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Payload`] when the text is not a JSON object.
    pub fn parse(text: &str) -> Result<Self, OperationError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|source| OperationError::Payload { source })?;
        let serde_json::Value::Object(object) = value else {
            return Err(OperationError::NotAnObject {
                what: "composition",
            });
        };
        if object.contains_key("_type") {
            Ok(Self::Canonical(object))
        } else {
            Ok(Self::Flat(object))
        }
    }

    /// Returns the members the payload carries.
    #[must_use]
    pub const fn members(&self) -> &serde_json::Map<String, serde_json::Value> {
        match *self {
            Self::Canonical(ref object) | Self::Flat(ref object) => object,
        }
    }

    /// Returns the template the payload names inline, when it carries one.
    ///
    /// "A canonical Composition carries its template inline at
    /// `archetype_details.template_id`" and a flat one carries none
    /// (`engine/rest-api.adoc` §Composition formats).
    #[must_use]
    pub fn template_id(&self) -> Option<TemplateId> {
        let Self::Canonical(ref object) = *self else {
            return None;
        };
        object
            .get("archetype_details")?
            .get("template_id")?
            .get("value")?
            .as_str()
            .map(TemplateId::new)
    }

    /// Returns the payload as the JSON string a `valueString` carries.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        serde_json::Value::Object(self.members().clone()).to_string()
    }
}

/// The serialization `$toopenehr` answers in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// The canonical Reference Model structure, "the form openEHR CDRs expect
    /// when a Composition is committed" and the default.
    #[default]
    Canonical,
    /// The single-level simSDT object.
    Flat,
}

impl Format {
    /// Returns the code the `format` parameter carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Flat => "flat",
        }
    }

    /// Returns the format `code` names.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Parameters`] when `code` is neither of the
    /// two the draft declares.
    pub fn parse(code: &str) -> Result<Self, OperationError> {
        match code {
            "canonical" => Ok(Self::Canonical),
            "flat" => Ok(Self::Flat),
            _ => Err(OperationError::Parameters {
                operation: TO_OPENEHR,
                source: ParametersError::WrongType {
                    operation: "$toopenehr",
                    name: FORMAT,
                    expected: "code of `canonical` or `flat`",
                },
            }),
        }
    }
}

/// The `in` parameters of `$tofhir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToFhirRequest {
    composition: CompositionPayload,
    template_id: Option<TemplateId>,
    context: Option<CallContext>,
}

impl ToFhirRequest {
    /// Creates a request carrying `composition` alone.
    #[must_use]
    pub const fn new(composition: CompositionPayload) -> Self {
        Self {
            composition,
            template_id: None,
            context: None,
        }
    }

    /// Returns this request with `template` pinning the mapping.
    #[must_use]
    pub fn with_template_id(mut self, template: TemplateId) -> Self {
        self.template_id = Some(template);
        self
    }

    /// Returns this request with `context` as its call context.
    #[must_use]
    pub fn with_context(mut self, context: CallContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Returns the composition the call carries.
    #[must_use]
    pub const fn composition(&self) -> &CompositionPayload {
        &self.composition
    }

    /// Returns the template the call pins, when it pins one.
    #[must_use]
    pub const fn template_id(&self) -> Option<&TemplateId> {
        self.template_id.as_ref()
    }

    /// Returns the call context, when the caller supplied one.
    #[must_use]
    pub const fn context(&self) -> Option<&CallContext> {
        self.context.as_ref()
    }

    /// Reads the request from a `Parameters` resource.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Parameters`] for an undeclared, repeated,
    /// missing or wrongly typed parameter, [`OperationError::Payload`] when
    /// the composition string is not a JSON object, and
    /// [`OperationError::FlatTemplate`] when a flat composition arrives with
    /// no `templateId`.
    pub fn from_parameters(parameters: &Parameters) -> Result<Self, OperationError> {
        let mut composition: Option<CompositionPayload> = None;
        let mut template_id: Option<TemplateId> = None;
        let mut context: Option<CallContext> = None;
        for parameter in &parameters.parameter {
            match named(TO_FHIR, "$tofhir", parameter)? {
                COMPOSITION => {
                    once(TO_FHIR, "$tofhir", COMPOSITION, composition.is_some())?;
                    let text = string(TO_FHIR, "$tofhir", COMPOSITION, parameter)?;
                    composition = Some(CompositionPayload::parse(text)?);
                }
                TEMPLATE_ID => {
                    once(TO_FHIR, "$tofhir", TEMPLATE_ID, template_id.is_some())?;
                    template_id = Some(TemplateId::new(string(
                        TO_FHIR,
                        "$tofhir",
                        TEMPLATE_ID,
                        parameter,
                    )?));
                }
                CONTEXT => {
                    once(TO_FHIR, "$tofhir", CONTEXT, context.is_some())?;
                    context = Some(call_context(parameter)?);
                }
                other => {
                    return Err(OperationError::Parameters {
                        operation: TO_FHIR,
                        source: ParametersError::Undeclared {
                            operation: "$tofhir",
                            name: String::from(other),
                        },
                    });
                }
            }
        }
        let composition = composition.ok_or(OperationError::Parameters {
            operation: TO_FHIR,
            source: ParametersError::Missing {
                operation: "$tofhir",
                name: COMPOSITION,
            },
        })?;
        if matches!(composition, CompositionPayload::Flat(_)) && template_id.is_none() {
            return Err(OperationError::FlatTemplate);
        }
        Ok(Self {
            composition,
            template_id,
            context,
        })
    }

    /// Writes the request as a `Parameters` resource.
    ///
    /// The parameters come out in the order `ToFhir.fsh` declares them.
    #[must_use]
    pub fn to_parameters(&self) -> Parameters {
        let mut parameter = vec![value_parameter(
            COMPOSITION,
            ParametersParameterValue::String(fhir_types::r4::primitives::String::from(
                self.composition.to_json_string(),
            )),
        )];
        if let Some(ref template) = self.template_id {
            parameter.push(value_parameter(
                TEMPLATE_ID,
                ParametersParameterValue::String(fhir_types::r4::primitives::String::from(
                    template.as_str(),
                )),
            ));
        }
        if let Some(ref carried) = self.context {
            parameter.push(context_parameter(carried));
        }
        Parameters {
            parameter,
            ..Parameters::default()
        }
    }
}

/// The `out` parameter of `$tofhir`.
///
/// "The response MUST be a FHIR `Bundle` containing the mapped resources"
/// (`engine/rest-api.adoc` §$tofhir Output), and `ToFhir.fsh` names it
/// `return`, so the framework returns the Bundle itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToFhirResponse {
    bundle: Bundle,
}

impl ToFhirResponse {
    /// Wraps the Bundle a run produced.
    #[must_use]
    pub const fn new(bundle: Bundle) -> Self {
        Self { bundle }
    }

    /// Returns the Bundle.
    #[must_use]
    pub const fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    /// Returns the Bundle, consuming the response.
    #[must_use]
    pub fn into_bundle(self) -> Bundle {
        self.bundle
    }
}

/// The `in` parameters of `$toopenehr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToOpenehrRequest {
    bundle: Bundle,
    template_id: Option<TemplateId>,
    format: Format,
}

impl ToOpenehrRequest {
    /// Creates a request carrying `bundle` alone, answered in `canonical`.
    #[must_use]
    pub fn new(bundle: Bundle) -> Self {
        Self {
            bundle,
            template_id: None,
            format: Format::default(),
        }
    }

    /// Returns this request with `template` pinning the mapping.
    #[must_use]
    pub fn with_template_id(mut self, template: TemplateId) -> Self {
        self.template_id = Some(template);
        self
    }

    /// Returns this request answered in `format`.
    #[must_use]
    pub const fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// Returns the Bundle to convert.
    #[must_use]
    pub const fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    /// Returns the template the call pins, when it pins one.
    #[must_use]
    pub const fn template_id(&self) -> Option<&TemplateId> {
        self.template_id.as_ref()
    }

    /// Returns the serialization the call asked for.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// Reads the request from a `Parameters` resource.
    ///
    /// `ToOpenEhr.fsh` declares `bundle`, `templateId` and `format` as `in`
    /// parameters, which is the enveloped form of the same call the chapter's
    /// prose describes as a bare `Bundle` body; the two do not agree, and both
    /// are read (recorded on FerroBRIDGE issue #192).
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Parameters`] for an undeclared, repeated,
    /// missing or wrongly typed parameter.
    pub fn from_parameters(parameters: &Parameters) -> Result<Self, OperationError> {
        let mut bundle: Option<Bundle> = None;
        let mut template_id: Option<TemplateId> = None;
        let mut format: Option<Format> = None;
        for parameter in &parameters.parameter {
            match named(TO_OPENEHR, "$toopenehr", parameter)? {
                BUNDLE => {
                    once(TO_OPENEHR, "$toopenehr", BUNDLE, bundle.is_some())?;
                    let Some(Resource::Bundle(ref carried)) = parameter.resource else {
                        return Err(OperationError::Parameters {
                            operation: TO_OPENEHR,
                            source: ParametersError::WrongType {
                                operation: "$toopenehr",
                                name: BUNDLE,
                                expected: "Bundle",
                            },
                        });
                    };
                    bundle = Some(carried.as_ref().clone());
                }
                TEMPLATE_ID => {
                    once(TO_OPENEHR, "$toopenehr", TEMPLATE_ID, template_id.is_some())?;
                    template_id = Some(TemplateId::new(string(
                        TO_OPENEHR,
                        "$toopenehr",
                        TEMPLATE_ID,
                        parameter,
                    )?));
                }
                FORMAT => {
                    once(TO_OPENEHR, "$toopenehr", FORMAT, format.is_some())?;
                    format = Some(Format::parse(code(
                        TO_OPENEHR,
                        "$toopenehr",
                        FORMAT,
                        parameter,
                    )?)?);
                }
                other => {
                    return Err(OperationError::Parameters {
                        operation: TO_OPENEHR,
                        source: ParametersError::Undeclared {
                            operation: "$toopenehr",
                            name: String::from(other),
                        },
                    });
                }
            }
        }
        let bundle = bundle.ok_or(OperationError::Parameters {
            operation: TO_OPENEHR,
            source: ParametersError::Missing {
                operation: "$toopenehr",
                name: BUNDLE,
            },
        })?;
        Ok(Self {
            bundle,
            template_id,
            format: format.unwrap_or_default(),
        })
    }

    /// Writes the request as a `Parameters` resource.
    #[must_use]
    pub fn to_parameters(&self) -> Parameters {
        let mut parameter = vec![ParametersParameter {
            name: fhir_types::r4::primitives::String::from(BUNDLE),
            resource: Some(Resource::Bundle(Box::new(self.bundle.clone()))),
            ..ParametersParameter::default()
        }];
        if let Some(ref template) = self.template_id {
            parameter.push(value_parameter(
                TEMPLATE_ID,
                ParametersParameterValue::String(fhir_types::r4::primitives::String::from(
                    template.as_str(),
                )),
            ));
        }
        parameter.push(value_parameter(
            FORMAT,
            ParametersParameterValue::Code(fhir_types::r4::primitives::Code::from(
                self.format.as_str(),
            )),
        ));
        Parameters {
            parameter,
            ..Parameters::default()
        }
    }
}

/// The `out` parameters of `$toopenehr`.
///
/// "The response MUST be a FHIR `Parameters` resource. On full success it
/// contains only the `composition` parameter" (`engine/rest-api.adoc`
/// §$toopenehr Output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToOpenehrResponse {
    composition: String,
    outcome: Option<OperationOutcome>,
}

impl ToOpenehrResponse {
    /// Wraps the composition a run produced.
    #[must_use]
    pub const fn new(composition: String) -> Self {
        Self {
            composition,
            outcome: None,
        }
    }

    /// Returns this response with `outcome` beside the composition.
    #[must_use]
    pub fn with_outcome(mut self, outcome: OperationOutcome) -> Self {
        self.outcome = Some(outcome);
        self
    }

    /// Returns the composition, serialized as the request asked for.
    #[must_use]
    pub fn composition(&self) -> &str {
        &self.composition
    }

    /// Returns the outcome, when the run declared a loss.
    #[must_use]
    pub const fn outcome(&self) -> Option<&OperationOutcome> {
        self.outcome.as_ref()
    }

    /// Reads the response from a `Parameters` resource.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Parameters`] for an undeclared, repeated,
    /// missing or wrongly typed parameter.
    pub fn from_parameters(parameters: &Parameters) -> Result<Self, OperationError> {
        let mut composition: Option<String> = None;
        let mut outcome: Option<OperationOutcome> = None;
        for parameter in &parameters.parameter {
            match named(TO_OPENEHR, "$toopenehr", parameter)? {
                COMPOSITION => {
                    once(TO_OPENEHR, "$toopenehr", COMPOSITION, composition.is_some())?;
                    composition = Some(String::from(string(
                        TO_OPENEHR,
                        "$toopenehr",
                        COMPOSITION,
                        parameter,
                    )?));
                }
                OUTCOME => {
                    once(TO_OPENEHR, "$toopenehr", OUTCOME, outcome.is_some())?;
                    let Some(Resource::OperationOutcome(ref carried)) = parameter.resource else {
                        return Err(OperationError::Parameters {
                            operation: TO_OPENEHR,
                            source: ParametersError::WrongType {
                                operation: "$toopenehr",
                                name: OUTCOME,
                                expected: "OperationOutcome",
                            },
                        });
                    };
                    outcome = Some(carried.as_ref().clone());
                }
                other => {
                    return Err(OperationError::Parameters {
                        operation: TO_OPENEHR,
                        source: ParametersError::Undeclared {
                            operation: "$toopenehr",
                            name: String::from(other),
                        },
                    });
                }
            }
        }
        let composition = composition.ok_or(OperationError::Parameters {
            operation: TO_OPENEHR,
            source: ParametersError::Missing {
                operation: "$toopenehr",
                name: COMPOSITION,
            },
        })?;
        Ok(Self {
            composition,
            outcome,
        })
    }

    /// Writes the response as a `Parameters` resource.
    #[must_use]
    pub fn to_parameters(&self) -> Parameters {
        let mut parameter = vec![value_parameter(
            COMPOSITION,
            ParametersParameterValue::String(fhir_types::r4::primitives::String::from(
                self.composition.as_str(),
            )),
        )];
        if let Some(ref outcome) = self.outcome {
            parameter.push(ParametersParameter {
                name: fhir_types::r4::primitives::String::from(OUTCOME),
                resource: Some(Resource::OperationOutcome(Box::new(outcome.clone()))),
                ..ParametersParameter::default()
            });
        }
        Parameters {
            parameter,
            ..Parameters::default()
        }
    }
}
