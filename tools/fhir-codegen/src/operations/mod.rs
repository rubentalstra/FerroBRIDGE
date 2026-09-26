// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Operation contracts: the terminology `OperationDefinition`s as typed
//! request and response shapes plus a runtime descriptor.
//!
//! Each terminology operation of a version yields a request struct (its `in`
//! parameters), a response struct (its `out` parameters), one struct per
//! multi-part parameter, and a `const` describing the exact parameter set the
//! version declares (<https://hl7.org/fhir/R4B/operationdefinition.html>), so
//! a server can refuse what the version does not define
//! (`spec-adherence.md`). The terminology ecosystem overlay
//! (`crate::ecosystem`) adds its parameters before lowering, each marked
//! with its source.

mod field;
mod render;

use std::fmt::{self, Write};

use crate::ecosystem::{self, ParameterSource};
use crate::fhir::{OperationDefinition, ParameterUse};
use crate::lower::VersionModule;
use crate::naming::{module_name, type_name};
use crate::operations::field::disambiguate;
use crate::operations::field::lower_field;
use crate::operations::field::pascal;
use crate::operations::render::collapse_whitespace;
use crate::operations::render::render_conversions;
use crate::operations::render::render_descriptor;
use crate::operations::render::render_struct;
use crate::operations::render::wrap;
use crate::snapshot::Max;

/// The module holding the shared descriptor types.
pub const DESCRIPTOR_MODULE: &str = "operation";

/// The choice enum every `Element`-typed parameter maps to.
pub const OPEN_TYPE_ENUM: &str = "ParametersParameterValue";

/// A failure while lowering an operation.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    /// A parameter has neither a type nor parts.
    #[error("{operation}: parameter {name} has neither a type nor parts")]
    Untyped {
        /// The operation code.
        operation: String,
        /// The parameter name.
        name: String,
    },
    /// A parameter names a type the version module does not emit.
    #[error("{operation}: parameter {name} has type {code}, which the module does not emit")]
    UnknownType {
        /// The operation code.
        operation: String,
        /// The parameter name.
        name: String,
        /// The type code.
        code: String,
    },
    /// A parameter's `max` is neither a number nor `*`.
    #[error("{operation}: parameter {name} has an invalid max {max:?}")]
    InvalidMax {
        /// The operation code.
        operation: String,
        /// The parameter name.
        name: String,
        /// The offending value.
        max: String,
    },
    /// Rendering to a string failed.
    #[error("rendering failed")]
    Render(#[from] fmt::Error),
}

/// One typed parameter.
/// How a parameter's value travels in `Parameters.parameter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    /// A data type: one variant of the open-type enum, named like the type.
    Value(String),
    /// A resource: `parameter.resource`, one variant of the resource enum.
    Resource(String),
    /// `Resource` itself: `parameter.resource`, any variant.
    AnyResource,
    /// `Element`: any value of the open-type enum.
    OpenType,
    /// A multi-part parameter: `parameter.part`.
    Parts,
}

/// One operation parameter as the module renders it: a struct field plus
/// the wire facts the conversions and the descriptor need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractField {
    /// The Rust field name.
    pub name: String,
    /// The FHIR parameter name.
    pub fhir_name: String,
    /// The parameter documentation.
    pub documentation: Option<String>,
    /// `in` or `out`.
    pub usage: ParameterUse,
    /// The minimum cardinality.
    pub min: u32,
    /// The maximum cardinality.
    pub max: Max,
    /// The FHIR type code, absent for a multi-part parameter.
    pub type_code: Option<String>,
    /// The Rust type path the field holds (without `Option`/`Vec`).
    pub rust_type: String,
    /// The invocation levels the parameter applies to (R5 `scope`).
    pub scope: Vec<String>,
    /// The nested parts, lowered.
    pub parts: Vec<ContractField>,
    /// The struct name of the nested parts, when there are any.
    pub part_struct: Option<String>,
    /// How the value travels on the wire.
    pub kind: FieldKind,
    /// Whether the field's type derives `Default` (so a required field of it
    /// does not stop the owning struct from deriving `Default`).
    pub defaultable: bool,
    /// Where the parameter comes from: the version, or the ecosystem overlay.
    pub source: ParameterSource,
    /// The other primitive variants a value of this field is read from: the
    /// primitives that specialize the declared one and share its scalar
    /// (`code` for a `string` parameter, `canonical` for a `uri` one), and the
    /// ones it specializes on the same terms (`uri` for a `canonical`
    /// parameter).
    pub accepts: Vec<String>,
    /// Whether the open-type variant this field travels in holds its value
    /// behind a `Box`, which every complex type does so the enum stays as
    /// narrow as its primitives.
    pub boxed: bool,
}

/// One operation's contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationContract {
    /// The resource the operation applies to, for example `CodeSystem`.
    pub resource: String,
    /// The operation code, for example `lookup`.
    pub code: String,
    /// The canonical URL of the `OperationDefinition`.
    pub url: String,
    /// The operation's own documentation.
    pub description: Option<String>,
    /// The request struct name.
    pub request: String,
    /// The response struct name.
    pub response: String,
    /// The descriptor constant name.
    pub descriptor: String,
    /// The module (file) name.
    pub module: String,
    /// Whether the operation is invoked at the system level.
    pub system: bool,
    /// Whether the operation is invoked at the type level.
    pub type_level: bool,
    /// Whether the operation is invoked on an instance.
    pub instance: bool,
    /// The `in` parameters.
    pub inputs: Vec<ContractField>,
    /// The `out` parameters.
    pub outputs: Vec<ContractField>,
}

impl OperationContract {
    /// Lowers `definition` against the types `module` emits.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError`] for a parameter without type or parts, a
    /// type the module does not emit, or an invalid `max`.
    pub fn lower(
        definition: &OperationDefinition,
        resource: &str,
        module: &VersionModule,
    ) -> Result<Self, OperationError> {
        let stem = format!("{}{}", type_name(resource), pascal(&definition.code));
        let request = format!("{stem}Request");
        let response = format!("{stem}Response");
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let defaultable = crate::render::defaultable(module);
        for parameter in &definition.parameter {
            let owner = match parameter.usage {
                ParameterUse::In => &request,
                ParameterUse::Out => &response,
            };
            let field = lower_field(parameter, owner, &definition.code, module, &defaultable)?;
            match parameter.usage {
                ParameterUse::In => inputs.push(field),
                ParameterUse::Out => outputs.push(field),
            }
        }
        disambiguate(&mut inputs);
        disambiguate(&mut outputs);
        Ok(Self {
            resource: resource.to_owned(),
            code: definition.code.clone(),
            url: definition.url.clone(),
            description: definition.description.clone(),
            request,
            response,
            descriptor: module_name(&stem).to_uppercase(),
            module: module_name(&stem),
            system: definition.system,
            type_level: definition.type_level,
            instance: definition.instance,
            inputs,
            outputs,
        })
    }

    /// Lowers `definition` with the terminology ecosystem overlay applied
    /// (`r6` is the R6 definition of the same operation, the source of the
    /// pre-adopted parameters), marking each added field with its source.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError`] as [`Self::lower`] does.
    pub fn lower_overlaid(
        definition: &OperationDefinition,
        resource: &str,
        module: &VersionModule,
        r6: Option<&OperationDefinition>,
    ) -> Result<Self, OperationError> {
        let (overlaid, added) = ecosystem::overlay(definition, r6);
        let mut contract = Self::lower(&overlaid, resource, module)?;
        for field in contract.inputs.iter_mut().chain(&mut contract.outputs) {
            if let Some(entry) = added
                .iter()
                .find(|a| a.usage == field.usage && a.name == field.fhir_name)
            {
                field.source = entry.source;
            }
            for part in &mut field.parts {
                let dotted = format!("{}.{}", field.fhir_name, part.fhir_name);
                if let Some(entry) = added
                    .iter()
                    .find(|a| a.usage == field.usage && a.name == dotted)
                {
                    part.source = entry.source;
                }
            }
        }
        Ok(contract)
    }
}

/// The version-neutral descriptor module, `src/operation.rs`.
#[must_use]
pub fn render_descriptor_module(banner: &str) -> String {
    let mut out = banner.to_owned();
    out.push_str(
        r"//! Runtime descriptors of the terminology operations.
//!
//! The exact parameter set each FHIR version declares
//! (<https://hl7.org/fhir/R4B/operationdefinition.html>) plus the terminology
//! ecosystem overlay (<https://hl7.org/fhir/uv/tx-ecosystem/requirements.html>),
//! each parameter marked with its source, so a server accepts nothing more
//! and nothing less.
//!
//! NOTE: a parameter is read from the primitive its definition declares and
//! from the primitives that share that one's scalar in either direction, so a
//! `canonical` parameter reads a `valueUri` too. No clause requires refusing
//! the wider spelling, and the GET form carries no type marker at all
//! (<https://hl7.org/fhir/R4B/operations.html>); the decision is recorded on
//! #352.

/// The direction of an operation parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterUse {
    /// An input parameter.
    In,
    /// An output parameter.
    Out,
}

/// Where a declared parameter comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterSource {
    /// The version's own `OperationDefinition`.
    Version,
    /// Pre-adopted from the FHIR R6 ballot for the terminology ecosystem.
    PreAdopted,
    /// Defined by the terminology ecosystem alone.
    Ecosystem,
}

/// How many values a parameter takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cardinality {
    /// The minimum number of values.
    pub min: u32,
    /// The maximum number of values; `None` is unbounded (`*`).
    pub max: Option<u32>,
}

impl Cardinality {
    /// Whether `count` values satisfy this cardinality.
    #[must_use]
    pub const fn admits(self, count: u32) -> bool {
        count >= self.min
            && match self.max {
                Some(max) => count <= max,
                None => true,
            }
    }
}

/// One declared parameter, with its parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    /// The parameter name as it appears in `Parameters.parameter.name`.
    pub name: &'static str,
    /// The direction.
    pub usage: ParameterUse,
    /// The cardinality.
    pub cardinality: Cardinality,
    /// The FHIR type code, `None` for a multi-part parameter.
    pub type_code: Option<&'static str>,
    /// The invocation levels the parameter applies to; empty means every level.
    pub scope: &'static [&'static str],
    /// The nested parts.
    pub parts: &'static [Parameter],
    /// Where the parameter comes from.
    pub source: ParameterSource,
}

impl Parameter {
    /// The part named `name`, if any.
    #[must_use]
    pub fn part(&self, name: &str) -> Option<&'static Parameter> {
        self.parts.iter().find(|part| part.name == name)
    }
}

/// One operation as its version declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    /// The canonical URL of the `OperationDefinition`.
    pub url: &'static str,
    /// The resource type the operation applies to.
    pub resource: &'static str,
    /// The code used to invoke the operation, without the `$`.
    pub code: &'static str,
    /// Whether the operation is invoked at the system level.
    pub system: bool,
    /// Whether the operation is invoked at the resource type level.
    pub type_level: bool,
    /// Whether the operation is invoked on a resource instance.
    pub instance: bool,
    /// Every declared parameter, `in` and `out`, in declaration order.
    pub parameters: &'static [Parameter],
}

impl Operation {
    /// The parameter named `name` with the given direction, if declared.
    #[must_use]
    pub fn parameter(&self, usage: ParameterUse, name: &str) -> Option<&'static Parameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.usage == usage && parameter.name == name)
    }

    /// The parameters of one direction, in declaration order.
    pub fn parameters_of(&self, usage: ParameterUse) -> impl Iterator<Item = &'static Parameter> {
        self.parameters.iter().filter(move |parameter| parameter.usage == usage)
    }
}

",
    );
    out.push_str(PARAMETERS_ERROR);
    out
}

/// The error the generated `from_parameters` conversions return.
const PARAMETERS_ERROR: &str = r#"
/// Why a `Parameters` resource does not fit an operation's declared
/// parameter set. Every variant names the operation and the parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParametersError {
    /// A parameter has no name.
    Unnamed {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
    },
    /// A parameter the operation does not declare.
    Undeclared {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The parameter name (dotted for a part).
        name: std::string::String,
    },
    /// A parameter with a maximum of one given more than once.
    Repeated {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The parameter name (dotted for a part).
        name: &'static str,
    },
    /// A required parameter is absent.
    Missing {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The parameter name (dotted for a part).
        name: &'static str,
    },
    /// A parameter carries neither a value nor a resource.
    MissingValue {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The parameter name (dotted for a part).
        name: &'static str,
    },
    /// A parameter's value is not of the declared type.
    WrongType {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The parameter name (dotted for a part).
        name: &'static str,
        /// The declared type.
        expected: &'static str,
    },
}

impl std::fmt::Display for ParametersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unnamed { operation } => {
                write!(f, "{operation}: a parameter has no name")
            }
            Self::Undeclared { operation, name } => {
                write!(f, "{operation}: parameter `{name}` is not declared")
            }
            Self::Repeated { operation, name } => {
                write!(f, "{operation}: parameter `{name}` is given more than once")
            }
            Self::Missing { operation, name } => {
                write!(f, "{operation}: parameter `{name}` is required")
            }
            Self::MissingValue { operation, name } => {
                write!(f, "{operation}: parameter `{name}` has no value")
            }
            Self::WrongType {
                operation,
                name,
                expected,
            } => write!(f, "{operation}: parameter `{name}` is not a {expected}"),
        }
    }
}

impl std::error::Error for ParametersError {}
"#;

/// The `operations/mod.rs` of a version module.
///
/// # Errors
///
/// Returns [`fmt::Error`] only if writing to the string fails, which `String` never does.
pub fn render_operations_mod(
    banner: &str,
    version: &str,
    contracts: &[OperationContract],
) -> Result<String, fmt::Error> {
    let mut out = banner.to_owned();
    writeln!(
        out,
        "//! The FHIR {version} terminology operations: typed request and response\n//! shapes and the descriptor of each operation's declared parameter set.\n"
    )?;
    for contract in contracts {
        writeln!(out, "pub mod {};", contract.module)?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "/// Every terminology operation this version declares, in module order."
    )?;
    writeln!(
        out,
        "pub const OPERATIONS: [&super::super::{DESCRIPTOR_MODULE}::Operation; {}] = [",
        contracts.len()
    )?;
    for contract in contracts {
        writeln!(out, "    &{}::{},", contract.module, contract.descriptor)?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

/// One operation's file.
///
/// # Errors
///
/// Returns [`fmt::Error`] only if writing to the string fails, which `String` never does.
pub fn render_operation(banner: &str, contract: &OperationContract) -> Result<String, fmt::Error> {
    let mut out = banner.to_owned();
    writeln!(out, "//! `{}/${}`.", contract.resource, contract.code)?;
    writeln!(out, "//!")?;
    let description = contract.description.as_deref().map_or_else(
        || String::from("(undocumented in the package)"),
        |d| crate::render::escape_doc(&collapse_whitespace(d)),
    );
    for line in wrap(&description) {
        writeln!(out, "//! {line}")?;
    }
    writeln!(out)?;
    render_struct(
        &mut out,
        &contract.request,
        &format!(
            "The `in` parameters of `{}/${}`.",
            contract.resource, contract.code
        ),
        &contract.inputs,
    )?;
    render_struct(
        &mut out,
        &contract.response,
        &format!(
            "The `out` parameters of `{}/${}`.",
            contract.resource, contract.code
        ),
        &contract.outputs,
    )?;
    let operation = format!("{}/${}", contract.resource, contract.code);
    render_conversions(
        &mut out,
        &contract.request,
        &operation,
        &contract.inputs,
        "",
    )?;
    render_conversions(
        &mut out,
        &contract.response,
        &operation,
        &contract.outputs,
        "",
    )?;
    render_descriptor(&mut out, contract)?;
    Ok(out)
}
