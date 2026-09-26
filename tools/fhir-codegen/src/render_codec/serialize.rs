// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The serde `Serialize` of a struct, run by run.

use std::fmt::{self, Write};

use crate::lower::{Cardinality, Field, Scalar, TypeDef, TypeKind, Variant, VersionModule};
use crate::naming::type_name;
use crate::render::{render_target, resource_scope};
use crate::render_codec::C;
use crate::render_codec::Shape;
use crate::render_codec::choice::holds_primitive;
use crate::render_codec::shape;

/// Which half of a field one JSON key carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The value key.
    Value,
    /// The `_name` sibling.
    Element,
}

/// One JSON key a type can write, and what writes it.
struct Slot<'a> {
    /// The property name.
    key: String,
    /// The field it comes from and that field's position, absent for
    /// `resourceType`.
    field: Option<(usize, &'a Field)>,
    /// Which half of the field.
    part: Part,
    /// The choice form, when the field is a choice element.
    variant: Option<&'a Variant>,
}

/// Every key `ty` can write, in the order the JSON object holds them.
///
// NOTE: the codec holds an object as a `BTreeMap`, which iterates in key order
// (<https://doc.rust-lang.org/std/collections/struct.BTreeMap.html>), so writing
// the keys sorted gives the direct path the bytes the document path produces.
fn slots<'a>(model: &'a VersionModule, ty: &'a TypeDef, fields: &'a [Field]) -> Vec<Slot<'a>> {
    let mut slots = Vec::new();
    if ty.is_resource {
        slots.push(Slot {
            key: String::from("resourceType"),
            field: None,
            part: Part::Value,
            variant: None,
        });
    }
    for (index, field) in fields.iter().enumerate() {
        let key = &field.fhir_name;
        let mut push = |key: String, part, variant| {
            slots.push(Slot {
                key,
                field: Some((index, field)),
                part,
                variant,
            });
        };
        match shape(model, field) {
            Shape::Scalar(_) | Shape::Complex => push(key.clone(), Part::Value, None),
            Shape::Primitive => {
                push(key.clone(), Part::Value, None);
                push(format!("_{key}"), Part::Element, None);
            }
            Shape::Choice(choice) => {
                let stem = key.trim_end_matches("[x]");
                if let TypeKind::Choice { variants, .. } = &choice.kind {
                    for variant in variants {
                        let suffix = type_name(&variant.code);
                        push(format!("{stem}{suffix}"), Part::Value, Some(variant));
                        if holds_primitive(model, variant) {
                            push(format!("_{stem}{suffix}"), Part::Element, Some(variant));
                        }
                    }
                }
            }
        }
    }
    slots.sort_by(|a, b| a.key.cmp(&b.key));
    slots
}

/// Whether two neighbouring keys are forms of one choice element, which one
/// `match` writes together.
fn one_choice(a: &Slot<'_>, b: &Slot<'_>) -> bool {
    a.variant.is_some()
        && b.variant.is_some()
        && a.part == b.part
        && a.field.map(|(index, _)| index) == b.field.map(|(index, _)| index)
}

/// One `serialize_entry` call.
fn entry(out: &mut String, indent: &str, key: &str, value: &str) -> fmt::Result {
    writeln!(
        out,
        "{indent}serde::ser::SerializeMap::serialize_entry(&mut map, {key:?}, {value})?;"
    )
}

/// The struct's `Serialize`: the object `to_json` builds, written straight to
/// the serializer.
pub(super) fn render_serialize(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    fields: &[Field],
) -> fmt::Result {
    let slots = slots(model, ty, fields);
    writeln!(out, "\nimpl serde::Serialize for {} {{", ty.name)?;
    writeln!(
        out,
        "    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{"
    )?;
    let binding = if slots.is_empty() { "let" } else { "let mut" };
    writeln!(
        out,
        "        {binding} map = serde::Serializer::serialize_map(serializer, None)?;"
    )?;
    for run in slots.chunk_by(one_choice) {
        render_run(model, out, ty, run)?;
    }
    writeln!(out, "        serde::ser::SerializeMap::end(map)")?;
    writeln!(out, "    }}\n}}")
}

/// One key, or the forms of one choice element that sort together.
fn render_run(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    run: &[Slot<'_>],
) -> fmt::Result {
    let Some(first) = run.first() else {
        return Ok(());
    };
    let Some((_, field)) = first.field else {
        return entry(out, "        ", "resourceType", &format!("{:?}", ty.name));
    };
    match shape(model, field) {
        Shape::Scalar(scalar) => render_scalar_entry(out, field, first, scalar),
        Shape::Primitive => render_primitive_entry(out, field, first),
        Shape::Complex => render_complex_entry(out, field, first),
        Shape::Choice(choice) => render_choice_run(model, out, ty, field, choice, run),
    }
}

/// The serializable expression for one scalar held at `place`, a reference.
fn scalar_serializable(scalar: Scalar, place: &str) -> String {
    match scalar {
        Scalar::I64 => format!("&{place}.to_string()"),
        _ => String::from(place),
    }
}

/// The serializable expression for a list of scalars at `access`.
fn scalar_list_serializable(scalar: Scalar, access: &str) -> String {
    match scalar {
        Scalar::I64 => format!(
            "&{access}.iter().map(std::string::ToString::to_string).collect::<Vec<std::string::String>>()"
        ),
        _ => format!("&{access}"),
    }
}

/// A `FHIRPath` system scalar: a bare JSON value under its own key.
fn render_scalar_entry(
    out: &mut String,
    field: &Field,
    slot: &Slot<'_>,
    scalar: Scalar,
) -> fmt::Result {
    let key = &slot.key;
    let access = format!("self.{}", field.name);
    match field.ty.card {
        Cardinality::One => entry(
            out,
            "        ",
            key,
            &scalar_serializable(scalar, &format!("&{access}")),
        ),
        Cardinality::Optional => {
            writeln!(out, "        if let Some(v) = &{access} {{")?;
            entry(out, "            ", key, &scalar_serializable(scalar, "v"))?;
            writeln!(out, "        }}")
        }
        Cardinality::Many => {
            writeln!(out, "        if !{access}.is_empty() {{")?;
            entry(
                out,
                "            ",
                key,
                &scalar_list_serializable(scalar, &access),
            )?;
            writeln!(out, "        }}")
        }
    }
}

/// A FHIR primitive: the value under its key, the `id` and `extension` under
/// the `_name` sibling (<https://hl7.org/fhir/R4B/json.html#primitive>).
fn render_primitive_entry(out: &mut String, field: &Field, slot: &Slot<'_>) -> fmt::Result {
    let key = &slot.key;
    let access = format!("self.{}", field.name);
    let writer = match (slot.part, field.ty.card) {
        (Part::Value, Cardinality::Many) => "value_list_entry",
        (Part::Element, Cardinality::Many) => "element_list_entry",
        (Part::Value, _) => "value_entry",
        (Part::Element, _) => "element_entry",
    };
    match field.ty.card {
        Cardinality::Optional => {
            writeln!(out, "        if let Some(item) = &{access} {{")?;
            writeln!(out, "            {C}::{writer}(&mut map, {key:?}, item)?;")?;
            writeln!(out, "        }}")
        }
        _ => writeln!(out, "        {C}::{writer}(&mut map, {key:?}, &{access})?;"),
    }
}

/// A complex type, a backbone element, or the resource enum: a JSON object.
fn render_complex_entry(out: &mut String, field: &Field, slot: &Slot<'_>) -> fmt::Result {
    let key = &slot.key;
    let access = format!("self.{}", field.name);
    match field.ty.card {
        Cardinality::One => {
            let inner = if field.ty.boxed {
                format!("{access}.as_ref()")
            } else {
                format!("&{access}")
            };
            entry(out, "        ", key, &inner)
        }
        Cardinality::Optional => {
            let inner = if field.ty.boxed {
                "item.as_ref()"
            } else {
                "item"
            };
            writeln!(out, "        if let Some(item) = &{access} {{")?;
            entry(out, "            ", key, inner)?;
            writeln!(out, "        }}")
        }
        Cardinality::Many => {
            writeln!(out, "        if !{access}.is_empty() {{")?;
            entry(out, "            ", key, &format!("&{access}"))?;
            writeln!(out, "        }}")
        }
    }
}

/// The forms of one choice element: the key names the form the value holds
/// (<https://hl7.org/fhir/R4B/formats.html#choice>).
fn render_choice_run(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    field: &Field,
    choice: &TypeDef,
    run: &[Slot<'_>],
) -> fmt::Result {
    let TypeKind::Choice { variants, .. } = &choice.kind else {
        return Ok(());
    };
    let path = render_target(model, &ty.module, &field.ty.target, false);
    let access = format!("self.{}", field.name);
    let (scrutinee, optional) = match (field.ty.card, field.ty.boxed) {
        (Cardinality::Optional, true) => (format!("{access}.as_deref()"), true),
        (Cardinality::Optional, false) => (format!("&{access}"), true),
        (_, true) => (format!("{access}.as_ref()"), false),
        (_, false) => (format!("&{access}"), false),
    };
    let written: Vec<&str> = run
        .iter()
        .filter_map(|slot| slot.variant.map(|variant| variant.name.as_str()))
        .collect();
    let missing: Vec<&Variant> = variants
        .iter()
        .filter(|variant| !written.contains(&variant.name.as_str()))
        .collect();
    let pattern = |variant: &Variant| {
        let form = format!("{path}::{}(inner)", variant.name);
        if optional {
            format!("Some({form})")
        } else {
            form
        }
    };
    if run.len() == 1 && (optional || !missing.is_empty()) {
        let Some(slot) = run.first() else {
            return Ok(());
        };
        let Some(variant) = slot.variant else {
            return Ok(());
        };
        writeln!(out, "        if let {} = {scrutinee} {{", pattern(variant))?;
        render_choice_arm(model, out, "            ", slot, variant)?;
        return writeln!(out, "        }}");
    }
    writeln!(out, "        match {scrutinee} {{")?;
    for slot in run {
        let Some(variant) = slot.variant else {
            continue;
        };
        writeln!(out, "            {} => {{", pattern(variant))?;
        render_choice_arm(model, out, "                ", slot, variant)?;
        writeln!(out, "            }}")?;
    }
    match (optional, missing.as_slice()) {
        (true, []) => writeln!(out, "            None => {{}}")?,
        (false, []) => {}
        // NOTE: a wildcard over one variant trips
        // `clippy::match_wildcard_for_single_variants`, so name it.
        (false, [one]) => writeln!(out, "            {path}::{}(_) => {{}}", one.name)?,
        _ => writeln!(out, "            _ => {{}}")?,
    }
    writeln!(out, "        }}")
}

/// What one choice form writes under its key.
fn render_choice_arm(
    model: &VersionModule,
    out: &mut String,
    indent: &str,
    slot: &Slot<'_>,
    variant: &Variant,
) -> fmt::Result {
    let key = &slot.key;
    if holds_primitive(model, variant) {
        let writer = match slot.part {
            Part::Value => "value_entry",
            Part::Element => "element_entry",
        };
        return writeln!(out, "{indent}{C}::{writer}(&mut map, {key:?}, inner)?;");
    }
    let inner = if variant.boxed {
        "inner.as_ref()"
    } else {
        "inner"
    };
    entry(out, indent, key, inner)
}

/// The resource enum's `Serialize`: the resource it holds writes itself.
pub(super) fn render_resource_serialize(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    resources: &[String],
) -> fmt::Result {
    writeln!(out, "\nimpl serde::Serialize for {} {{", ty.name)?;
    writeln!(
        out,
        "    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{"
    )?;
    writeln!(out, "        match self {{")?;
    for resource in resources {
        out.push_str(&resource_scope(model, resource).cfg());
        writeln!(
            out,
            "            Self::{resource}(inner) => serde::Serialize::serialize(inner.as_ref(), serializer),"
        )?;
    }
    writeln!(
        out,
        "            Self::Unknown(inner) => match &inner.body {{"
    )?;
    writeln!(
        out,
        "                {C}::Value::Object(object) => serde::Serialize::serialize(object, serializer),"
    )?;
    writeln!(
        out,
        "                _ => Err(<S::Error as serde::ser::Error>::custom({C}::EncodeError::UnknownResourceBody)),"
    )?;
    writeln!(out, "            }},")?;
    writeln!(out, "        }}\n    }}\n}}")
}
