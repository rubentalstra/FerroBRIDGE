// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The JSON conversion of a struct, field by field.

use std::fmt::{self, Write};

use crate::lower::{Cardinality, Field, TypeDef, TypeKind, VersionModule};
use crate::naming::type_name;
use crate::render_codec::C;
use crate::render_codec::Shape;
use crate::render_codec::builder::render_field_builders;
use crate::render_codec::scalar_to_value;
use crate::render_codec::scalar_to_value_owned;
use crate::render_codec::shape;

pub(super) fn render_struct(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    fields: &[Field],
) -> fmt::Result {
    writeln!(out, "\nimpl {C}::Json for {} {{", ty.name)?;
    render_to_json(model, out, ty, fields)?;
    writeln!(out)?;
    render_from_json(model, out, ty, fields)?;
    writeln!(out, "}}")
}

fn render_to_json(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    fields: &[Field],
) -> fmt::Result {
    writeln!(
        out,
        "    fn to_json(&self) -> Result<{C}::Object, {C}::EncodeError> {{"
    )?;
    writeln!(out, "        let mut object = {C}::Object::new();")?;
    if ty.is_resource {
        writeln!(
            out,
            "        object.insert(std::string::String::from(\"resourceType\"), {C}::Value::String(std::string::String::from({:?})));",
            ty.name
        )?;
    }
    for field in fields {
        render_field_to_json(model, out, field)?;
    }
    writeln!(out, "        Ok(object)")?;
    writeln!(out, "    }}")
}

/// Writes one field into the object under construction.
#[expect(
    clippy::too_many_lines,
    reason = "one match arm per FHIR field shape and cardinality; splitting scatters the table"
)]
fn render_field_to_json(model: &VersionModule, out: &mut String, field: &Field) -> fmt::Result {
    let key = &field.fhir_name;
    let access = format!("self.{}", field.name);
    match (shape(model, field), field.ty.card) {
        (Shape::Scalar(scalar), Cardinality::Optional) => {
            writeln!(out, "        if let Some(v) = &{access} {{")?;
            writeln!(
                out,
                "            object.insert(std::string::String::from({key:?}), {});",
                scalar_to_value(scalar, "v")
            )?;
            writeln!(out, "        }}")?;
        }
        (Shape::Scalar(scalar), Cardinality::One) => {
            writeln!(
                out,
                "        object.insert(std::string::String::from({key:?}), {});",
                scalar_to_value_owned(scalar, &access)
            )?;
        }
        (Shape::Scalar(scalar), Cardinality::Many) => {
            writeln!(out, "        if !{access}.is_empty() {{")?;
            writeln!(
                out,
                "            object.insert(std::string::String::from({key:?}), {C}::Value::Array({access}.iter().map(|v| {}).collect()));",
                scalar_to_value(scalar, "v")
            )?;
            writeln!(out, "        }}")?;
        }
        (Shape::Primitive, Cardinality::One | Cardinality::Optional) => {
            let (open, close, item) = if field.ty.card == Cardinality::Optional {
                (
                    format!("        if let Some(item) = &{access} {{\n"),
                    "        }\n",
                    String::from("item"),
                )
            } else {
                (String::new(), "", format!("&{access}"))
            };
            let indent = if field.ty.card == Cardinality::Optional {
                "            "
            } else {
                "        "
            };
            out.push_str(&open);
            writeln!(
                out,
                "{indent}if let Some(v) = {C}::Primitive::value_json({item})? {{"
            )?;
            writeln!(
                out,
                "{indent}    object.insert(std::string::String::from({key:?}), v);"
            )?;
            writeln!(out, "{indent}}}")?;
            writeln!(
                out,
                "{indent}if let Some(e) = {C}::Primitive::element_json({item})? {{"
            )?;
            writeln!(
                out,
                "{indent}    object.insert(std::string::String::from(\"_{key}\"), e);"
            )?;
            writeln!(out, "{indent}}}")?;
            out.push_str(close);
        }
        (Shape::Primitive, Cardinality::Many) => {
            writeln!(out, "        if !{access}.is_empty() {{")?;
            writeln!(
                out,
                "            let (values, elements) = {C}::primitive_arrays(&{access})?;"
            )?;
            writeln!(
                out,
                "            object.insert(std::string::String::from({key:?}), values);"
            )?;
            writeln!(out, "            if let Some(elements) = elements {{")?;
            writeln!(
                out,
                "                object.insert(std::string::String::from(\"_{key}\"), elements);"
            )?;
            writeln!(out, "            }}")?;
            writeln!(out, "        }}")?;
        }
        (Shape::Complex, Cardinality::One) => {
            let inner = if field.ty.boxed {
                format!("{access}.as_ref()")
            } else {
                format!("&{access}")
            };
            writeln!(
                out,
                "        object.insert(std::string::String::from({key:?}), {C}::Value::Object({C}::Json::to_json({inner})?));"
            )?;
        }
        (Shape::Complex, Cardinality::Optional) => {
            let inner = if field.ty.boxed {
                "item.as_ref()"
            } else {
                "item"
            };
            writeln!(out, "        if let Some(item) = &{access} {{")?;
            writeln!(
                out,
                "            object.insert(std::string::String::from({key:?}), {C}::Value::Object({C}::Json::to_json({inner})?));"
            )?;
            writeln!(out, "        }}")?;
        }
        (Shape::Complex, Cardinality::Many) => {
            writeln!(out, "        if !{access}.is_empty() {{")?;
            writeln!(
                out,
                "            let mut items = Vec::with_capacity({access}.len());"
            )?;
            writeln!(out, "            for item in &{access} {{")?;
            writeln!(
                out,
                "                items.push({C}::Value::Object({C}::Json::to_json(item)?));"
            )?;
            writeln!(out, "            }}")?;
            writeln!(
                out,
                "            object.insert(std::string::String::from({key:?}), {C}::Value::Array(items));"
            )?;
            writeln!(out, "        }}")?;
        }
        (Shape::Choice(_), card) => {
            let stem = key.trim_end_matches("[x]");
            let (open, close, item, indent) = match card {
                Cardinality::Optional => (
                    format!("        if let Some(item) = &{access} {{\n"),
                    "        }\n",
                    String::from("item"),
                    "            ",
                ),
                _ => (String::new(), "", access.clone(), "        "),
            };
            out.push_str(&open);
            writeln!(
                out,
                "{indent}let (suffix, value, element) = {item}.to_json_parts()?;"
            )?;
            writeln!(out, "{indent}if let Some(value) = value {{")?;
            writeln!(
                out,
                "{indent}    object.insert(format!(\"{stem}{{suffix}}\"), value);"
            )?;
            writeln!(out, "{indent}}}")?;
            writeln!(out, "{indent}if let Some(element) = element {{")?;
            writeln!(
                out,
                "{indent}    object.insert(format!(\"_{stem}{{suffix}}\"), element);"
            )?;
            writeln!(out, "{indent}}}")?;
            out.push_str(close);
        }
    }
    Ok(())
}

fn render_from_json(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    fields: &[Field],
) -> fmt::Result {
    writeln!(
        out,
        "    fn from_json(object: &{C}::Object, path: &mut {C}::Path) -> Result<Self, {C}::DecodeError> {{"
    )?;
    // Raw slots: one per field, holding borrowed JSON until the loop finishes.
    for field in fields {
        let slot = &field.name.trim_start_matches("r#");
        match shape(model, field) {
            Shape::Choice(_) => writeln!(
                out,
                "        let mut raw_{slot} = {C}::ChoiceSlot::default();"
            )?,
            Shape::Primitive => {
                writeln!(
                    out,
                    "        let mut raw_{slot}: Option<&{C}::Value> = None;"
                )?;
                writeln!(
                    out,
                    "        let mut raw_{slot}_element: Option<&{C}::Value> = None;"
                )?;
            }
            Shape::Scalar(_) | Shape::Complex => writeln!(
                out,
                "        let mut raw_{slot}: Option<&{C}::Value> = None;"
            )?,
        }
    }
    writeln!(out, "        for (key, value) in object {{")?;
    writeln!(out, "            match key.as_str() {{")?;
    if ty.is_resource {
        writeln!(out, "                \"resourceType\" => {{")?;
        writeln!(
            out,
            "                    if value.as_str() != Some({:?}) {{",
            ty.name
        )?;
        writeln!(
            out,
            "                        return Err(path.error({C}::DecodeErrorKind::ResourceType));"
        )?;
        writeln!(out, "                    }}")?;
        writeln!(out, "                }}")?;
    }
    for field in fields {
        let slot = &field.name.trim_start_matches("r#");
        let key = &field.fhir_name;
        match shape(model, field) {
            Shape::Choice(choice) => {
                let stem = key.trim_end_matches("[x]");
                if let TypeKind::Choice { variants, .. } = &choice.kind {
                    for variant in variants {
                        let suffix = type_name(&variant.code);
                        writeln!(
                            out,
                            "                \"{stem}{suffix}\" => {{ raw_{slot}.value({suffix:?}, value, path)?; }}"
                        )?;
                        writeln!(
                            out,
                            "                \"_{stem}{suffix}\" => {{ raw_{slot}.element({suffix:?}, value, path)?; }}"
                        )?;
                    }
                }
            }
            Shape::Primitive => {
                writeln!(
                    out,
                    "                {key:?} => {{{{ raw_{slot} = Some(value); }}}}"
                )?;
                writeln!(
                    out,
                    "                \"_{key}\" => {{{{ raw_{slot}_element = Some(value); }}}}"
                )?;
            }
            Shape::Scalar(_) | Shape::Complex => {
                writeln!(
                    out,
                    "                {key:?} => {{{{ raw_{slot} = Some(value); }}}}"
                )?;
            }
        }
    }
    writeln!(
        out,
        "                other => return path.with(other, |path| Err(path.error({C}::DecodeErrorKind::UnknownProperty))),"
    )?;
    writeln!(out, "            }}")?;
    writeln!(out, "        }}")?;
    render_field_builders(model, out, fields)?;
    writeln!(out, "        Ok(Self {{")?;
    for field in fields {
        writeln!(
            out,
            "            {}: field_{},",
            field.name,
            field.name.trim_start_matches("r#")
        )?;
    }
    writeln!(out, "        }})")?;
    writeln!(out, "    }}")
}
