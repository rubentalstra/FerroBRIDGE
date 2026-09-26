// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lexical form of a string primitive: its anchored pattern and the check
//! that refuses a value outside it.

use std::fmt::{self, Write};

use crate::lower::{PRIMITIVES_MODULE, Scalar, TypeDef, VersionModule, anchored_lexical_form};
use crate::naming::module_name;
use crate::render_codec::C;

/// The lexical form `ty` states for its value, for a primitive the FHIR JSON
/// representation carries as a string.
///
/// A primitive carried as a JSON number or a boolean
/// (<https://hl7.org/fhir/R4B/json.html#primitive>) is decoded by the parser of
/// that scalar, which admits exactly the values the form describes, so the form
/// is read for the string-valued primitives alone.
pub(super) fn lexical_form(ty: &TypeDef, scalar: Scalar) -> Option<&str> {
    if !matches!(scalar, Scalar::Str) || ty.name == "Decimal" {
        return None;
    }
    ty.value_regex.as_deref()
}

/// The `LazyLock` holding the compiled lexical form of the primitive `name`.
fn lexical_form_static(name: &str) -> String {
    format!("{}_LEXICAL_FORM", module_name(name).to_uppercase())
}

/// The function checking a value against the lexical form of the primitive `name`.
pub(super) fn lexical_form_check(name: &str) -> String {
    format!("checked_{}", module_name(name))
}

/// That function, spelled from another module of the same version.
pub(super) fn lexical_form_path(name: &str) -> String {
    format!("super::{PRIMITIVES_MODULE}::{}", lexical_form_check(name))
}

/// The pattern as a Rust string literal, raw wherever the text admits one so
/// the backslashes read as the package writes them.
fn pattern_literal(pattern: &str) -> String {
    if pattern.contains('"') {
        format!("{pattern:?}")
    } else {
        format!("r\"{pattern}\"")
    }
}

/// The compiled lexical form and the function that checks a value against it.
pub(super) fn render_lexical_form(
    model: &VersionModule,
    out: &mut String,
    ty: &TypeDef,
    pattern: &str,
) -> fmt::Result {
    let form = lexical_form_static(&ty.name);
    let check = lexical_form_check(&ty.name);
    let version = model.name.to_uppercase();
    writeln!(
        out,
        "\n/// The lexical form of the FHIR primitive `{}`, anchored to the whole value.",
        ty.path
    )?;
    writeln!(out, "///")?;
    writeln!(
        out,
        "/// The `regex` extension of `{}.value` states it (<https://hl7.org/fhir/{version}/datatypes.html#primitive>),",
        ty.path
    )?;
    writeln!(
        out,
        "/// and the form is an XML Schema pattern, where `\\s` is the space, the tab,"
    )?;
    writeln!(
        out,
        "/// the carriage return and the line feed alone (<https://www.w3.org/TR/xmlschema-2/#regexs>)."
    )?;
    writeln!(out, "#[expect(")?;
    writeln!(out, "    clippy::expect_used,")?;
    writeln!(
        out,
        "    reason = \"the emitter compiles every lexical form it writes, so the pattern holds\""
    )?;
    writeln!(out, ")]")?;
    writeln!(
        out,
        "static {form}: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {{"
    )?;
    writeln!(
        out,
        "    regex::bytes::RegexBuilder::new({})",
        pattern_literal(&anchored_lexical_form(pattern))
    )?;
    writeln!(out, "        .unicode(false)")?;
    writeln!(out, "        .build()")?;
    writeln!(
        out,
        "        .expect(\"the lexical form of `{}` should compile\")",
        ty.path
    )?;
    writeln!(out, "}});")?;
    writeln!(
        out,
        "\n/// Hands back `text` when it keeps the lexical form of `{}`.",
        ty.path
    )?;
    writeln!(out, "///")?;
    writeln!(out, "/// # Errors")?;
    writeln!(out, "///")?;
    writeln!(
        out,
        "/// Returns [`{C}::DecodeErrorKind::BadValue`] at `path` for a value outside the form."
    )?;
    writeln!(out, "pub(crate) fn {check}(")?;
    writeln!(out, "    text: std::string::String,")?;
    writeln!(out, "    path: &{C}::Path,")?;
    writeln!(out, ") -> Result<std::string::String, {C}::DecodeError> {{")?;
    writeln!(out, "    if {form}.is_match(text.as_bytes()) {{")?;
    writeln!(out, "        Ok(text)")?;
    writeln!(out, "    }} else {{")?;
    writeln!(
        out,
        "        Err(path.error({C}::DecodeErrorKind::BadValue))"
    )?;
    writeln!(out, "    }}")?;
    writeln!(out, "}}")
}
