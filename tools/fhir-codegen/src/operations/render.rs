// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request and response structs of one operation and their conversions.

use std::fmt::{self, Write};

use crate::fhir::ParameterUse;
use crate::lower::Cardinality;
use crate::operations::ContractField;
use crate::operations::DESCRIPTOR_MODULE;
use crate::operations::FieldKind;
use crate::operations::OPEN_TYPE_ENUM;
use crate::operations::OperationContract;
use crate::operations::field::cardinality;
use crate::operations::field::derives_default;
use crate::snapshot::Max;

/// `from_parameters`/`into_parameters`/`to_parameters` on a request or response
/// struct, and the parameter-list pair on it and every part struct.
///
/// The moving form is the answer path; the borrowing one clones into it, so the
/// values are copied only where a caller genuinely keeps its own.
pub(super) fn render_conversions(
    out: &mut String,
    name: &str,
    operation: &str,
    fields: &[ContractField],
    prefix: &str,
) -> fmt::Result {
    writeln!(out, "impl {name} {{")?;
    if prefix.is_empty() {
        writeln!(
            out,
            "    /// Reads the parameters from a `Parameters` resource."
        )?;
        writeln!(out, "    ///")?;
        writeln!(out, "    /// # Errors")?;
        writeln!(out, "    ///")?;
        writeln!(
            out,
            "    /// Returns the error for an undeclared, repeated, missing, or wrongly typed parameter."
        )?;
        writeln!(
            out,
            "    pub fn from_parameters(parameters: &{P}::Parameters) -> Result<Self, {E}> {{"
        )?;
        writeln!(
            out,
            "        Self::from_parameter_list(&parameters.parameter)"
        )?;
        writeln!(out, "    }}")?;
        writeln!(
            out,
            "    /// Writes the parameters as a `Parameters` resource, by move."
        )?;
        writeln!(out, "    #[must_use]")?;
        writeln!(
            out,
            "    pub fn into_parameters(self) -> {P}::Parameters {{"
        )?;
        writeln!(out, "        {P}::Parameters {{")?;
        writeln!(out, "            parameter: self.into_parameter_list(),")?;
        writeln!(out, "            ..Default::default()")?;
        writeln!(out, "        }}")?;
        writeln!(out, "    }}")?;
        writeln!(
            out,
            "    /// Writes the parameters as a `Parameters` resource, from a reference."
        )?;
        writeln!(out, "    #[must_use]")?;
        writeln!(out, "    pub fn to_parameters(&self) -> {P}::Parameters {{")?;
        writeln!(out, "        self.clone().into_parameters()")?;
        writeln!(out, "    }}")?;
    }
    render_from_list(out, operation, fields, prefix)?;
    render_to_list(out, fields)?;
    writeln!(out, "}}\n")?;
    for field in fields {
        if let Some(nested) = &field.part_struct {
            render_conversions(
                out,
                nested,
                operation,
                &field.parts,
                &format!("{prefix}{}.", field.fhir_name),
            )?;
        }
    }
    Ok(())
}

const P: &str = "super::super::parameters";

const E: &str = "super::super::super::operation::ParametersError";

/// `from_parameter_list`: every field collected by name, cardinality enforced.
///
/// Locals carry a `field_` prefix so a parameter named `name` or `value`
/// cannot shadow the loop's bindings.
fn render_from_list(
    out: &mut String,
    operation: &str,
    fields: &[ContractField],
    prefix: &str,
) -> fmt::Result {
    writeln!(out, "    /// Reads the fields from a parameter list.")?;
    writeln!(out, "    ///")?;
    writeln!(out, "    /// # Errors")?;
    writeln!(out, "    ///")?;
    writeln!(
        out,
        "    /// Returns the error for an undeclared, repeated, missing, or wrongly typed parameter."
    )?;
    writeln!(
        out,
        "    pub fn from_parameter_list(list: &[{P}::ParametersParameter]) -> Result<Self, {E}> {{"
    )?;
    writeln!(out, "        const OPERATION: &str = {operation:?};")?;
    writeln!(out, "        const PREFIX: &str = {prefix:?};")?;
    for field in fields {
        if cardinality(field.min, field.max) == Cardinality::Many {
            writeln!(
                out,
                "        let mut {}: Vec<{}> = Vec::new();",
                local(field),
                field.rust_type
            )?;
        } else {
            writeln!(
                out,
                "        let mut {}: Option<{}> = None;",
                local(field),
                field.rust_type
            )?;
        }
    }
    writeln!(out, "        for parameter in list {{")?;
    writeln!(
        out,
        "            let parameter_name = parameter.name.value.as_deref().ok_or({E}::Unnamed {{ operation: OPERATION }})?;"
    )?;
    writeln!(out, "            match parameter_name {{")?;
    for field in fields {
        let path = format!("{prefix}{}", field.fhir_name);
        let extract = render_extract(field, &path);
        writeln!(out, "                {:?} => {{", field.fhir_name)?;
        if cardinality(field.min, field.max) == Cardinality::Many {
            writeln!(out, "                    {}.push({extract});", local(field))?;
        } else {
            writeln!(out, "                    if {}.is_some() {{", local(field))?;
            writeln!(
                out,
                "                        return Err({E}::Repeated {{ operation: OPERATION, name: {path:?} }});"
            )?;
            writeln!(out, "                    }}")?;
            writeln!(
                out,
                "                    {} = Some({extract});",
                local(field)
            )?;
        }
        writeln!(out, "                }}")?;
    }
    writeln!(out, "                other => {{")?;
    writeln!(
        out,
        "                    return Err({E}::Undeclared {{ operation: OPERATION, name: [PREFIX, other].concat() }});"
    )?;
    writeln!(out, "                }}")?;
    writeln!(out, "            }}")?;
    writeln!(out, "        }}")?;
    writeln!(out, "        Ok(Self {{")?;
    for field in fields {
        let path = format!("{prefix}{}", field.fhir_name);
        if cardinality(field.min, field.max) == Cardinality::One {
            writeln!(
                out,
                "            {}: {}.ok_or({E}::Missing {{ operation: OPERATION, name: {path:?} }})?,",
                field.name,
                local(field)
            )?;
        } else {
            writeln!(out, "            {}: {},", field.name, local(field))?;
        }
    }
    writeln!(out, "        }})")?;
    writeln!(out, "    }}")
}

/// The local that accumulates a field inside `from_parameter_list`: the
/// field name with a `field_` prefix and without a raw-identifier marker.
fn local(field: &ContractField) -> String {
    format!("field_{}", field.name.trim_start_matches("r#"))
}

/// `to_parameter_list`: one `ParametersParameter` per present value.
fn render_to_list(out: &mut String, fields: &[ContractField]) -> fmt::Result {
    writeln!(
        out,
        "    /// Writes the fields as a parameter list, by move."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn into_parameter_list(self) -> Vec<{P}::ParametersParameter> {{"
    )?;
    writeln!(
        out,
        "        let mut out = Vec::with_capacity({});",
        fields.len()
    )?;
    for field in fields {
        let build = render_build(field, "value");
        match cardinality(field.min, field.max) {
            Cardinality::One => {
                writeln!(out, "        {{")?;
                writeln!(out, "            let value = self.{};", field.name)?;
                writeln!(out, "            out.push({build});")?;
                writeln!(out, "        }}")?;
            }
            Cardinality::Optional => {
                writeln!(out, "        if let Some(value) = self.{} {{", field.name)?;
                writeln!(out, "            out.push({build});")?;
                writeln!(out, "        }}")?;
            }
            Cardinality::Many => {
                writeln!(out, "        for value in self.{} {{", field.name)?;
                writeln!(out, "            out.push({build});")?;
                writeln!(out, "        }}")?;
            }
        }
    }
    writeln!(out, "        out")?;
    writeln!(out, "    }}")?;
    writeln!(
        out,
        "    /// Writes the fields as a parameter list, from a reference."
    )?;
    writeln!(out, "    #[must_use]")?;
    writeln!(
        out,
        "    pub fn to_parameter_list(&self) -> Vec<{P}::ParametersParameter> {{"
    )?;
    writeln!(out, "        self.clone().into_parameter_list()")?;
    writeln!(out, "    }}")
}

/// The expression reading one field's value out of `parameter`.
fn render_extract(field: &ContractField, path: &str) -> String {
    let (p, e) = (P, E);
    match &field.kind {
        FieldKind::Value(variant) => {
            // NOTE: a value sent as a specialization of the declared primitive (a
            // `code` for a `string`, a `canonical` for a `uri`) reads as the
            // declared one (<https://hl7.org/fhir/R5/datatypes.html#primitive>).
            let accepted: Vec<String> = field
                .accepts
                .iter()
                .map(|accepted| {
                    format!(
                        "Some({p}::{OPEN_TYPE_ENUM}::{accepted}(value)) => {} {{ id: value.id.clone(), extension: value.extension.clone(), value: value.value.clone() }}, ",
                        field.rust_type
                    )
                })
                .collect();
            let own = if field.boxed {
                "(**value).clone()"
            } else {
                "value.clone()"
            };
            let arms = format!(
                "Some({p}::{OPEN_TYPE_ENUM}::{variant}(value)) => {own}, {}",
                accepted.join("")
            );
            format!(
                "match &parameter.value {{ {arms}Some(_) => return Err({e}::WrongType {{ operation: OPERATION, name: {path:?}, expected: {variant:?} }}), None => return Err({e}::MissingValue {{ operation: OPERATION, name: {path:?} }}) }}"
            )
        }
        FieldKind::Resource(resource) => format!(
            "match &parameter.resource {{ Some(super::super::resource::Resource::{resource}(value)) => (**value).clone(), Some(_) => return Err({e}::WrongType {{ operation: OPERATION, name: {path:?}, expected: {resource:?} }}), None => return Err({e}::MissingValue {{ operation: OPERATION, name: {path:?} }}) }}"
        ),
        FieldKind::OpenType => format!(
            "parameter.value.clone().ok_or({e}::MissingValue {{ operation: OPERATION, name: {path:?} }})?"
        ),
        FieldKind::AnyResource => format!(
            "parameter.resource.clone().ok_or({e}::MissingValue {{ operation: OPERATION, name: {path:?} }})?"
        ),
        FieldKind::Parts => format!("{}::from_parameter_list(&parameter.part)?", field.rust_type),
    }
}

/// The expression building one `ParametersParameter` out of `value`, which it
/// takes by move: the answer is built once and handed on, never copied.
fn render_build(field: &ContractField, value: &str) -> String {
    let p = P;
    let name = format!("{:?}.into()", field.fhir_name);
    match &field.kind {
        FieldKind::Value(variant) => {
            let held = if field.boxed {
                format!("Box::new({value})")
            } else {
                value.to_owned()
            };
            format!(
                "{p}::ParametersParameter {{ name: {name}, value: Some({p}::{OPEN_TYPE_ENUM}::{variant}({held})), ..Default::default() }}"
            )
        }
        FieldKind::Resource(resource) => format!(
            "{p}::ParametersParameter {{ name: {name}, resource: Some(super::super::resource::Resource::{resource}(Box::new({value}))), ..Default::default() }}"
        ),
        FieldKind::OpenType => format!(
            "{p}::ParametersParameter {{ name: {name}, value: Some({value}), ..Default::default() }}"
        ),
        FieldKind::AnyResource => format!(
            "{p}::ParametersParameter {{ name: {name}, resource: Some({value}), ..Default::default() }}"
        ),
        FieldKind::Parts => format!(
            "{p}::ParametersParameter {{ name: {name}, part: {value}.into_parameter_list(), ..Default::default() }}"
        ),
    }
}

pub(super) fn render_struct(
    out: &mut String,
    name: &str,
    doc: &str,
    fields: &[ContractField],
) -> fmt::Result {
    writeln!(out, "/// {doc}")?;
    if derives_default(fields) {
        out.push_str("#[derive(Debug, Clone, Default, PartialEq, Eq)]\n");
    } else {
        out.push_str("#[derive(Debug, Clone, PartialEq, Eq)]\n");
    }
    writeln!(out, "pub struct {name} {{")?;
    for field in fields {
        let doc = field.documentation.as_deref().map_or_else(
            || format!("The `{}` parameter.", field.fhir_name),
            crate::render::escape_doc,
        );
        for line in wrap(&doc) {
            writeln!(out, "    /// {line}")?;
        }
        let ty = match cardinality(field.min, field.max) {
            Cardinality::One => field.rust_type.clone(),
            Cardinality::Optional => format!("Option<{}>", field.rust_type),
            Cardinality::Many => format!("Vec<{}>", field.rust_type),
        };
        writeln!(out, "    pub {}: {ty},", field.name)?;
    }
    out.push_str("}\n\n");
    for field in fields {
        if let Some(nested) = &field.part_struct {
            render_struct(
                out,
                nested,
                &format!("The parts of the `{}` parameter.", field.fhir_name),
                &field.parts,
            )?;
        }
    }
    Ok(())
}

pub(super) fn render_descriptor(out: &mut String, contract: &OperationContract) -> fmt::Result {
    let d = format!("super::super::super::{DESCRIPTOR_MODULE}");
    writeln!(
        out,
        "/// The declared parameter set of `{}/${}`.",
        contract.resource, contract.code
    )?;
    writeln!(
        out,
        "pub const {}: {d}::Operation = {d}::Operation {{",
        contract.descriptor
    )?;
    writeln!(out, "    url: {:?},", contract.url)?;
    writeln!(out, "    resource: {:?},", contract.resource)?;
    writeln!(out, "    code: {:?},", contract.code)?;
    writeln!(out, "    system: {},", contract.system)?;
    writeln!(out, "    type_level: {},", contract.type_level)?;
    writeln!(out, "    instance: {},", contract.instance)?;
    out.push_str("    parameters: &[\n");
    for field in contract.inputs.iter().chain(&contract.outputs) {
        render_parameter(out, field, &d, 2)?;
    }
    out.push_str("    ],\n};\n");
    Ok(())
}

fn render_parameter(out: &mut String, field: &ContractField, d: &str, depth: usize) -> fmt::Result {
    let pad = "    ".repeat(depth);
    writeln!(out, "{pad}{d}::Parameter {{")?;
    writeln!(out, "{pad}    name: {:?},", field.fhir_name)?;
    let usage = match field.usage {
        ParameterUse::In => "In",
        ParameterUse::Out => "Out",
    };
    writeln!(out, "{pad}    usage: {d}::ParameterUse::{usage},")?;
    let max = match field.max {
        Max::Unbounded => String::from("None"),
        Max::Bounded(n) => format!("Some({n})"),
    };
    writeln!(
        out,
        "{pad}    cardinality: {d}::Cardinality {{ min: {}, max: {max} }},",
        field.min
    )?;
    match &field.type_code {
        Some(code) => writeln!(out, "{pad}    type_code: Some({code:?}),")?,
        None => writeln!(out, "{pad}    type_code: None,")?,
    }
    let scope: Vec<String> = field.scope.iter().map(|s| format!("{s:?}")).collect();
    writeln!(out, "{pad}    scope: &[{}],", scope.join(", "))?;
    writeln!(
        out,
        "{pad}    source: {d}::ParameterSource::{},",
        field.source.variant()
    )?;
    if field.parts.is_empty() {
        writeln!(out, "{pad}    parts: &[],")?;
    } else {
        writeln!(out, "{pad}    parts: &[")?;
        for part in &field.parts {
            render_parameter(out, part, d, depth + 2)?;
        }
        writeln!(out, "{pad}    ],")?;
    }
    writeln!(out, "{pad}}},")
}

pub(super) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > 72 {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
