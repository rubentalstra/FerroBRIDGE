// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The codec of a primitive type and its element form.

use std::fmt::{self, Write};

use crate::lower::{Cardinality, Field, Scalar, Target, TypeDef, VersionModule};
use crate::render_codec::C;
use crate::render_codec::builder::inline_lexical_form;
use crate::render_codec::lexical::lexical_form;
use crate::render_codec::lexical::lexical_form_check;
use crate::render_codec::lexical::render_lexical_form;
use crate::render_codec::scalar_from_value;
use crate::render_codec::scalar_to_value;

/// What a primitive's codec needs to know of its own shape.
struct PrimitiveShape {
    /// The Rust scalar the value holds.
    scalar: Scalar,
    /// The function holding the value to its lexical form, where the package
    /// states one for the primitive.
    value_check: Option<String>,
    /// The same for the element's `id`, where the package states a form for
    /// the type that element names.
    id_check: Option<String>,
    /// Whether the value element is required.
    value_required: bool,
    /// Whether the primitive carries an `id`.
    has_id: bool,
    /// Whether the primitive carries extensions.
    has_extension: bool,
}

pub(super) fn render_primitive(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    fields: &[Field],
) -> fmt::Result {
    let value_field = fields.iter().find(|f| f.name == "value");
    let scalar = value_field
        .and_then(|f| match f.ty.target {
            Target::Inline(s) => Some(s),
            Target::Named(_) => None,
        })
        .unwrap_or(Scalar::Str);
    let form = lexical_form(ty, scalar);
    let shape = PrimitiveShape {
        scalar,
        value_check: form.map(|_| lexical_form_check(&ty.name)),
        id_check: fields
            .iter()
            .find(|f| f.name == "id")
            .and_then(|f| inline_lexical_form(model, f, Scalar::Str)),
        value_required: value_field.is_some_and(|f| f.ty.card == Cardinality::One),
        // R6 prohibits `xhtml.id` (max 0), so a primitive may carry no id at all.
        has_id: fields.iter().any(|f| f.name == "id"),
        has_extension: fields.iter().any(|f| f.name == "extension"),
    };
    if let Some(pattern) = form {
        render_lexical_form(model, out, ty, pattern)?;
    }
    writeln!(out, "\nimpl {C}::Primitive for {} {{", ty.name)?;
    writeln!(
        out,
        "    fn value_json(&self) -> Result<Option<{C}::Value>, {C}::EncodeError> {{"
    )?;
    let held = if shape.value_required {
        "Some(&self.value)"
    } else {
        "self.value.as_ref()"
    };
    match (scalar, ty.name.as_str()) {
        (Scalar::Str, "Decimal") => {
            writeln!(out, "        {held}.map(|text| {{")?;
            writeln!(out, "            text.parse::<{C}::Number>()")?;
            writeln!(out, "                .map({C}::Value::Number)")?;
            writeln!(
                out,
                "                .map_err(|_| {C}::EncodeError::BadDecimal {{ text: text.clone() }})"
            )?;
            writeln!(out, "        }}).transpose()")?;
        }
        _ => writeln!(
            out,
            "        Ok({held}.map(|v| {}))",
            scalar_to_value(scalar, "v")
        )?,
    }
    writeln!(out, "    }}\n")?;
    render_primitive_element_json(out, &shape)?;
    render_primitive_serialize(out, ty, &shape)?;
    render_primitive_decode(out, ty, &shape)?;
    writeln!(out, "}}")
}

/// The `element_json` writer: the `id` and `extension` of the `_name` sibling
/// (<https://hl7.org/fhir/R4B/json.html#primitive>).
fn render_primitive_element_json(out: &mut String, shape: &PrimitiveShape) -> fmt::Result {
    let PrimitiveShape {
        has_id,
        has_extension,
        ..
    } = *shape;
    writeln!(
        out,
        "    fn element_json(&self) -> Result<Option<{C}::Value>, {C}::EncodeError> {{"
    )?;
    match (has_id, has_extension) {
        (true, true) => writeln!(
            out,
            "        if self.id.is_none() && self.extension.is_empty() {{"
        )?,
        (true, false) => writeln!(out, "        if self.id.is_none() {{")?,
        (false, true) => writeln!(out, "        if self.extension.is_empty() {{")?,
        (false, false) => {}
    }
    if has_id || has_extension {
        writeln!(out, "            return Ok(None);")?;
        writeln!(out, "        }}")?;
        writeln!(out, "        let mut object = {C}::Object::new();")?;
    } else {
        // Nothing but the value: the element form never carries anything.
        writeln!(out, "        Ok(None)")?;
    }
    if has_id {
        writeln!(out, "        if let Some(id) = &self.id {{")?;
        writeln!(
            out,
            "            object.insert(std::string::String::from(\"id\"), {C}::Value::String(id.clone()));"
        )?;
        writeln!(out, "        }}")?;
    }
    if has_extension {
        writeln!(out, "        if !self.extension.is_empty() {{")?;
        writeln!(
            out,
            "            let mut items = Vec::with_capacity(self.extension.len());"
        )?;
        writeln!(out, "            for item in &self.extension {{")?;
        writeln!(
            out,
            "                items.push({C}::Value::Object({C}::Json::to_json(item)?));"
        )?;
        writeln!(out, "            }}")?;
        writeln!(
            out,
            "            object.insert(std::string::String::from(\"extension\"), {C}::Value::Array(items));"
        )?;
        writeln!(out, "        }}")?;
    }
    if has_id || has_extension {
        writeln!(out, "        Ok(Some({C}::Value::Object(object)))")?;
    }
    writeln!(out, "    }}\n")
}

/// The scalar's serializer call over `serializer` and `place`, a reference to
/// the held value.
fn scalar_serialize_call(scalar: Scalar, place: &str) -> String {
    match scalar {
        Scalar::Bool => format!("serde::Serializer::serialize_bool(serializer, *{place})"),
        Scalar::I32 => format!("serde::Serializer::serialize_i32(serializer, *{place})"),
        Scalar::U32 => format!("serde::Serializer::serialize_u32(serializer, *{place})"),
        Scalar::I64 => {
            format!("serde::Serializer::serialize_str(serializer, &{place}.to_string())")
        }
        Scalar::Str => format!("serde::Serializer::serialize_str(serializer, {place})"),
    }
}

/// The primitive's two direct writers: the value and the `_name` object.
fn render_primitive_serialize(
    out: &mut String,
    ty: &TypeDef,
    shape: &PrimitiveShape,
) -> fmt::Result {
    let PrimitiveShape {
        scalar,
        value_required,
        has_id,
        has_extension,
        ..
    } = *shape;
    writeln!(out, "    fn has_value(&self) -> bool {{")?;
    if value_required {
        writeln!(out, "        true")?;
    } else {
        writeln!(out, "        self.value.is_some()")?;
    }
    writeln!(out, "    }}\n")?;
    let held = match (has_id, has_extension) {
        (true, true) => "self.id.is_some() || !self.extension.is_empty()",
        (true, false) => "self.id.is_some()",
        (false, true) => "!self.extension.is_empty()",
        (false, false) => "false",
    };
    writeln!(out, "    fn has_element(&self) -> bool {{")?;
    writeln!(out, "        {held}")?;
    writeln!(out, "    }}\n")?;
    writeln!(
        out,
        "    fn serialize_value<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{"
    )?;
    render_primitive_value_body(out, ty, scalar, value_required)?;
    writeln!(out, "    }}\n")?;
    writeln!(
        out,
        "    fn serialize_element<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{"
    )?;
    match (has_id, has_extension) {
        (true, true) => writeln!(
            out,
            "        if self.id.is_none() && self.extension.is_empty() {{"
        )?,
        (true, false) => writeln!(out, "        if self.id.is_none() {{")?,
        (false, true) => writeln!(out, "        if self.extension.is_empty() {{")?,
        (false, false) => {}
    }
    if has_id || has_extension {
        writeln!(
            out,
            "            return serde::Serializer::serialize_none(serializer);"
        )?;
        writeln!(out, "        }}")?;
        writeln!(
            out,
            "        let mut map = serde::Serializer::serialize_map(serializer, None)?;"
        )?;
        if has_extension {
            writeln!(out, "        if !self.extension.is_empty() {{")?;
            writeln!(
                out,
                "            serde::ser::SerializeMap::serialize_entry(&mut map, \"extension\", &self.extension)?;"
            )?;
            writeln!(out, "        }}")?;
        }
        if has_id {
            writeln!(out, "        if let Some(id) = &self.id {{")?;
            writeln!(
                out,
                "            serde::ser::SerializeMap::serialize_entry(&mut map, \"id\", id)?;"
            )?;
            writeln!(out, "        }}")?;
        }
        writeln!(out, "        serde::ser::SerializeMap::end(map)")?;
    } else {
        writeln!(out, "        serde::Serializer::serialize_none(serializer)")?;
    }
    writeln!(out, "    }}\n")
}

/// The value writer's body: the decimal keeps its lexical form
/// (<https://hl7.org/fhir/R4B/json.html>), every other primitive writes its
/// scalar.
fn render_primitive_value_body(
    out: &mut String,
    ty: &TypeDef,
    scalar: Scalar,
    value_required: bool,
) -> fmt::Result {
    let decimal = matches!(scalar, Scalar::Str) && ty.name == "Decimal";
    if value_required {
        if decimal {
            return render_decimal_value(out, "        ", "&self.value");
        }
        return writeln!(
            out,
            "        {}",
            scalar_serialize_call(scalar, "&self.value")
        );
    }
    writeln!(out, "        match self.value.as_ref() {{")?;
    if decimal {
        writeln!(out, "            Some(text) => {{")?;
        render_decimal_value(out, "                ", "text")?;
        writeln!(out, "            }}")?;
    } else {
        writeln!(
            out,
            "            Some(v) => {},",
            scalar_serialize_call(scalar, "v")
        )?;
    }
    writeln!(
        out,
        "            None => serde::Serializer::serialize_none(serializer),"
    )?;
    writeln!(out, "        }}")
}

/// The decimal's value writer over `text`, its lexical form.
fn render_decimal_value(out: &mut String, indent: &str, text: &str) -> fmt::Result {
    writeln!(out, "{indent}serde::Serialize::serialize(")?;
    writeln!(
        out,
        "{indent}    &{text}.parse::<{C}::Number>().map_err(|_| {{"
    )?;
    writeln!(
        out,
        "{indent}        <S::Error as serde::ser::Error>::custom({C}::EncodeError::BadDecimal {{ text: {text}.clone() }})"
    )?;
    writeln!(out, "{indent}    }})?,")?;
    writeln!(out, "{indent}    serializer,")?;
    writeln!(out, "{indent})")
}

/// The head of `from_json_parts`: the signature and the decoded `value`.
///
/// A lenient path reads an absent required primitive as the value-less element
/// an extension-only `_name` sibling would produce
/// (<https://hl7.org/fhir/R4B/validation.html>).
fn render_primitive_decode_value(
    out: &mut String,
    ty: &TypeDef,
    shape: &PrimitiveShape,
) -> fmt::Result {
    let PrimitiveShape {
        scalar,
        value_required,
        ..
    } = *shape;
    writeln!(out, "    fn from_json_parts(")?;
    writeln!(out, "        value: Option<&{C}::Value>,")?;
    writeln!(out, "        element: Option<&{C}::Value>,")?;
    writeln!(out, "        path: &mut {C}::Path,")?;
    writeln!(out, "    ) -> Result<Self, {C}::DecodeError> {{")?;
    writeln!(
        out,
        "        if value.is_none() && element.is_none() && !path.is_lenient() {{"
    )?;
    writeln!(
        out,
        "            return Err(path.error({C}::DecodeErrorKind::MissingProperty));"
    )?;
    writeln!(out, "        }}")?;
    writeln!(out, "        let value = match value {{")?;
    writeln!(out, "            Some(value) => {{")?;
    writeln!(
        out,
        "                let value = {C}::expect_single(value, path)?;"
    )?;
    let decoded = scalar_from_value(scalar, Some(ty.name.as_str()));
    match shape.value_check.as_deref() {
        Some(check) => writeln!(out, "                Some({check}({decoded}?, path)?)")?,
        None => writeln!(out, "                Some({decoded}?)")?,
    }
    writeln!(out, "            }}")?;
    writeln!(out, "            None => None,")?;
    writeln!(out, "        }};")?;
    if value_required {
        writeln!(
            out,
            "        let value = value.ok_or_else(|| path.error({C}::DecodeErrorKind::MissingProperty))?;"
        )?;
    }
    Ok(())
}

fn render_primitive_decode(out: &mut String, ty: &TypeDef, shape: &PrimitiveShape) -> fmt::Result {
    let PrimitiveShape {
        has_id,
        has_extension,
        ..
    } = *shape;
    let id_check = shape.id_check.as_deref();
    render_primitive_decode_value(out, ty, shape)?;
    if has_id {
        writeln!(out, "        let mut id = None;")?;
    }
    if has_extension {
        writeln!(out, "        let mut extension = Vec::new();")?;
    }
    writeln!(out, "        if let Some(element) = element {{")?;
    writeln!(
        out,
        "            let object = {C}::expect_object(element, path)?;"
    )?;
    if !has_id && !has_extension {
        // No element property is admitted: the first key is the error.
        writeln!(
            out,
            "            if let Some(key) = object.keys().next() {{"
        )?;
        writeln!(
            out,
            "                return path.with(key, |path| Err(path.error({C}::DecodeErrorKind::UnknownProperty)));"
        )?;
        writeln!(out, "            }}")?;
        writeln!(out, "        }}")?;
        writeln!(out, "        Ok(Self {{ value }})")?;
        writeln!(out, "    }}")?;
        return Ok(());
    }
    writeln!(out, "            for (key, item) in object {{")?;
    writeln!(out, "                match key.as_str() {{")?;
    if has_id {
        let read = match id_check {
            Some(check) => format!("{check}({C}::expect_string(item, path)?, path)"),
            None => format!("{C}::expect_string(item, path)"),
        };
        writeln!(
            out,
            "                    \"id\" => {{ id = Some(path.with(\"id\", |path| {read})?); }}"
        )?;
    }
    if has_extension {
        writeln!(out, "                    \"extension\" => {{")?;
        writeln!(
            out,
            "                        for (index, entry) in {C}::expect_array(item, path)?.iter().enumerate() {{"
        )?;
        writeln!(
            out,
            "                            extension.push(path.with_index(\"extension\", index, |path| {{"
        )?;
        writeln!(
            out,
            "                                {C}::Json::from_json({C}::expect_object(entry, path)?, path)"
        )?;
        writeln!(out, "                            }})?);")?;
        writeln!(out, "                        }}")?;
        writeln!(out, "                    }}")?;
    }
    writeln!(
        out,
        "                    _ => return path.with(key, |path| Err(path.error({C}::DecodeErrorKind::UnknownProperty))),"
    )?;
    writeln!(out, "                }}")?;
    writeln!(out, "            }}")?;
    writeln!(out, "        }}")?;
    let mut init = Vec::new();
    if has_id {
        init.push("id");
    }
    if has_extension {
        init.push("extension");
    }
    init.push("value");
    writeln!(out, "        Ok(Self {{ {} }})", init.join(", "))?;
    writeln!(out, "    }}")
}
