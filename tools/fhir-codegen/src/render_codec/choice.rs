// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Choice types and the resource enum.

use std::fmt::{self, Write};

use crate::lower::{Target, TypeDef, Variant, VersionModule};
use crate::naming::type_name;
use crate::render::resource_scope;
use crate::render_codec::C;

pub(super) fn render_choice(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    variants: &[Variant],
) -> fmt::Result {
    writeln!(out, "\nimpl {} {{", ty.name)?;
    render_choice_to_json_parts(model, out, variants)?;
    render_choice_from_json_parts(model, out, variants)?;
    writeln!(out, "}}")
}

/// Whether the variant holds a FHIR primitive, which travels as value plus
/// `_name`.
pub(super) fn holds_primitive(model: &VersionModule, variant: &Variant) -> bool {
    matches!(&variant.target, Target::Named(name) if model.types.get(name).is_some_and(|ty| ty.is_primitive))
}

/// The choice enum's `to_json_parts`, one match arm per variant.
fn render_choice_to_json_parts(
    model: &VersionModule,
    out: &mut String,
    variants: &[Variant],
) -> fmt::Result {
    writeln!(
        out,
        "    /// The key suffix, the value part, and the `_name` part of this form."
    )?;
    writeln!(out, "    ///")?;
    writeln!(out, "    /// # Errors")?;
    writeln!(out, "    ///")?;
    writeln!(
        out,
        "    /// Returns [`{C}::EncodeError`] when a held value has no JSON form."
    )?;
    writeln!(
        out,
        "    pub fn to_json_parts(&self) -> Result<(&'static str, Option<{C}::Value>, Option<{C}::Value>), {C}::EncodeError> {{"
    )?;
    writeln!(out, "        match self {{")?;
    for variant in variants {
        let suffix = type_name(&variant.code);
        if holds_primitive(model, variant) {
            writeln!(
                out,
                "            Self::{}(inner) => Ok(({suffix:?}, {C}::Primitive::value_json(inner)?, {C}::Primitive::element_json(inner)?)),",
                variant.name
            )?;
        } else {
            let deref = if variant.boxed {
                "inner.as_ref()"
            } else {
                "inner"
            };
            writeln!(
                out,
                "            Self::{}(inner) => Ok(({suffix:?}, Some({C}::Value::Object({C}::Json::to_json({deref})?)), None)),",
                variant.name
            )?;
        }
    }
    writeln!(out, "        }}\n    }}\n")
}

/// The choice enum's `from_json_parts`, one match arm per variant suffix.
fn render_choice_from_json_parts(
    model: &VersionModule,
    out: &mut String,
    variants: &[Variant],
) -> fmt::Result {
    writeln!(
        out,
        "    /// Decodes the form named by `suffix` from its value and `_name` parts."
    )?;
    writeln!(out, "    ///")?;
    writeln!(out, "    /// # Errors")?;
    writeln!(out, "    ///")?;
    writeln!(
        out,
        "    /// Returns [`{C}::DecodeError`] for an unknown suffix or a malformed part."
    )?;
    writeln!(out, "    pub fn from_json_parts(")?;
    writeln!(out, "        suffix: &str,")?;
    writeln!(out, "        value: Option<&{C}::Value>,")?;
    writeln!(out, "        element: Option<&{C}::Value>,")?;
    writeln!(out, "        path: &mut {C}::Path,")?;
    writeln!(out, "    ) -> Result<Self, {C}::DecodeError> {{")?;
    writeln!(out, "        match suffix {{")?;
    for variant in variants {
        let suffix = type_name(&variant.code);
        if holds_primitive(model, variant) {
            writeln!(
                out,
                "            {suffix:?} => Ok(Self::{}({C}::Primitive::from_json_parts(value, element, path)?)),",
                variant.name
            )?;
        } else {
            let wrap = if variant.boxed {
                "Box::new(inner)"
            } else {
                "inner"
            };
            writeln!(out, "            {suffix:?} => {{")?;
            writeln!(out, "                if element.is_some() {{")?;
            writeln!(
                out,
                "                    return Err(path.error({C}::DecodeErrorKind::WrongType {{ expected: \"no underscore form for a complex type\" }}));"
            )?;
            writeln!(out, "                }}")?;
            writeln!(
                out,
                "                let value = value.ok_or_else(|| path.error({C}::DecodeErrorKind::MissingProperty))?;"
            )?;
            writeln!(
                out,
                "                let inner = {C}::Json::from_json({C}::expect_object({C}::expect_single(value, path)?, path)?, path)?;"
            )?;
            writeln!(out, "                Ok(Self::{}({wrap}))", variant.name)?;
            writeln!(out, "            }}")?;
        }
    }
    writeln!(
        out,
        "            _ => Err(path.error({C}::DecodeErrorKind::UnknownProperty)),"
    )?;
    writeln!(out, "        }}\n    }}")
}

pub(super) fn render_resource_enum(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    resources: &[String],
) -> fmt::Result {
    writeln!(out, "\nimpl {C}::Json for {} {{", ty.name)?;
    writeln!(
        out,
        "    fn to_json(&self) -> Result<{C}::Object, {C}::EncodeError> {{"
    )?;
    writeln!(out, "        match self {{")?;
    for resource in resources {
        out.push_str(&resource_scope(model, resource).cfg());
        writeln!(
            out,
            "            Self::{resource}(inner) => {C}::Json::to_json(inner.as_ref()),"
        )?;
    }
    writeln!(
        out,
        "            Self::Unknown(inner) => match &inner.body {{"
    )?;
    writeln!(
        out,
        "                {C}::Value::Object(object) => Ok(object.clone()),"
    )?;
    writeln!(
        out,
        "                _ => Err({C}::EncodeError::UnknownResourceBody),"
    )?;
    writeln!(out, "            }},")?;
    writeln!(out, "        }}\n    }}\n")?;
    writeln!(
        out,
        "    fn from_json(object: &{C}::Object, path: &mut {C}::Path) -> Result<Self, {C}::DecodeError> {{"
    )?;
    writeln!(out, "        match {C}::resource_type(object, path)? {{")?;
    for resource in resources {
        out.push_str(&resource_scope(model, resource).cfg());
        writeln!(
            out,
            "            {resource:?} => Ok(Self::{resource}({C}::boxed(object, path)?)),"
        )?;
    }
    writeln!(
        out,
        "            other => Ok(Self::Unknown(UnknownResource {{"
    )?;
    writeln!(out, "                resource_type: other.to_owned(),")?;
    writeln!(
        out,
        "                body: {C}::Value::Object(object.clone()),"
    )?;
    writeln!(out, "            }})),")?;
    writeln!(out, "        }}\n    }}\n}}")
}
