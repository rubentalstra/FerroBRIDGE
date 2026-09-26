// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The environment overrides, applied over the file before the tree is read.

use std::collections::BTreeMap;

use crate::telemetry::{FILTER_ENV, FORMAT_ENV};

use super::ENV_PREFIX;
use super::Error;

/// Applies one environment override onto `table`.
pub(super) fn apply_override(table: &mut toml::Table, name: &str, raw: &str) -> Result<(), Error> {
    let Some(path) = name.strip_prefix(ENV_PREFIX) else {
        return Ok(());
    };
    let segments: Vec<String> = path.split("__").map(str::to_ascii_lowercase).collect();
    let Some((key, parents)) = segments.split_last() else {
        return Err(Error::EnvName {
            name: name.to_owned(),
        });
    };
    if key.is_empty() || parents.is_empty() || parents.iter().any(String::is_empty) {
        return Err(Error::EnvName {
            name: name.to_owned(),
        });
    }
    let mut cursor = table;
    for parent in parents {
        let entry = cursor
            .entry(parent.clone())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let toml::Value::Table(next) = entry else {
            return Err(Error::EnvShape {
                name: name.to_owned(),
            });
        };
        cursor = next;
    }
    cursor.insert(key.clone(), env_value(raw));
    Ok(())
}

/// Applies [`FORMAT_ENV`] and [`FILTER_ENV`] onto `[telemetry]`.
///
/// They run after every `FERROBRIDGE__` override, so they win over the file
/// and over `FERROBRIDGE__TELEMETRY__FORMAT` and `FERROBRIDGE__TELEMETRY__FILTER`:
/// they are the names an operator sets for one run. Each value is text, so a
/// format outside `auto`, `json` and `pretty` is refused by the parse that
/// follows, naming `telemetry.format`.
pub(super) fn apply_console_overrides(
    table: &mut toml::Table,
    environment: &BTreeMap<String, String>,
) -> Result<(), Error> {
    for (variable, key) in [(FORMAT_ENV, "format"), (FILTER_ENV, "filter")] {
        let Some(raw) = environment.get(variable) else {
            continue;
        };
        let entry = table
            .entry("telemetry")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let toml::Value::Table(telemetry) = entry else {
            return Err(Error::EnvShape {
                name: variable.to_owned(),
            });
        };
        telemetry.insert(key.to_owned(), toml::Value::String(raw.clone()));
    }
    Ok(())
}

/// Reads `raw` as a TOML value, or as the string it is.
///
/// An environment variable carries text, so a number, a boolean and an array
/// are spelled in TOML syntax and everything else is the string itself. A
/// value that would read as another type is quoted.
fn env_value(raw: &str) -> toml::Value {
    // NOTE: no specification governs this: our own design. A parse failure IS
    // the answer here, because text that is not TOML syntax is a plain string.
    let parsed = toml::from_str::<toml::Table>(&format!("value = {raw}"))
        .ok()
        .and_then(|table| table.get("value").cloned());
    parsed.unwrap_or_else(|| toml::Value::String(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{apply_override, env_value};
    use crate::config::Error;

    #[test]
    fn an_environment_value_reads_as_toml_syntax_or_as_the_string_it_is() {
        assert_eq!(toml::Value::Integer(5), env_value("5"));
        assert_eq!(toml::Value::Boolean(true), env_value("true"));
        assert_eq!(
            toml::Value::String(String::from("http://cdr.invalid/v1")),
            env_value("http://cdr.invalid/v1")
        );
        assert_eq!(
            toml::Value::String(String::from("0.0.0.0:8080")),
            env_value("0.0.0.0:8080")
        );
    }

    #[test]
    fn an_override_name_without_a_section_and_a_key_is_refused() {
        let mut table = toml::Table::new();
        let error = apply_override(&mut table, "FERROBRIDGE__LISTEN", "x")
            .expect_err("a section and a key are both required");
        assert!(matches!(error, Error::EnvName { .. }), "{error:?}");
    }
}
