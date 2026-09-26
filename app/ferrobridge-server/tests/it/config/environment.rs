// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The environment over the file: the tree overrides and the console
//! variables.

use ferrobridge_server::config::{Config, Error};
use ferrobridge_server::telemetry::{DEFAULT_FILTER, FILTER_ENV, FORMAT_ENV, Format};
use std::collections::BTreeMap;
use std::error::Error as StdError;

use super::FULL;
use super::env;

#[test]
fn an_environment_override_wins_over_the_file() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(
        Some(FULL),
        &env("FERROBRIDGE__SERVER__LISTEN", "127.0.0.1:7777"),
    )?
    .resolve()?;
    assert_eq!("127.0.0.1:7777", settings.server.listen.to_string());
    Ok(())
}

#[test]
fn an_environment_override_reaches_a_nested_section_and_keeps_its_type()
-> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(
        Some(FULL),
        &env("FERROBRIDGE__CDR__RETRY__MAX_ATTEMPTS", "9"),
    )?
    .resolve()?;
    let cdr = settings.cdr.as_ref().ok_or("the CDR lane is on")?;
    assert_eq!(9, cdr.retry.max_attempts);
    Ok(())
}

#[test]
fn an_environment_override_turns_a_lane_on_with_no_file_at_all() -> Result<(), Box<dyn StdError>> {
    let environment = BTreeMap::from([
        (
            String::from("FERROBRIDGE__CDR__BASE_URL"),
            String::from("http://cdr.invalid/v1"),
        ),
        (
            String::from("FERROBRIDGE__CDR__CREDENTIALS__BEARER_TOKEN"),
            String::from("synthetic-token"),
        ),
    ]);
    let settings = Config::from_sources(None, &environment)?.resolve()?;
    let cdr = settings.cdr.as_ref().ok_or("the CDR lane is on")?;
    assert!(cdr.credentials.is_some(), "the token reached the client");
    assert!(
        settings.terminology.is_none(),
        "a lane with no section stays off"
    );
    assert!(settings.cdm.is_none());
    Ok(())
}

#[test]
fn the_console_variables_win_over_the_file_and_the_tree_overrides() -> Result<(), Box<dyn StdError>>
{
    let file = "[telemetry]\nformat = \"json\"\nfilter = \"warn\"\n";
    let tree = BTreeMap::from([
        (
            String::from("FERROBRIDGE__TELEMETRY__FORMAT"),
            String::from("auto"),
        ),
        (
            String::from("FERROBRIDGE__TELEMETRY__FILTER"),
            String::from("error"),
        ),
    ]);
    let config = Config::from_sources(Some(file), &tree)?;
    assert_eq!(Format::Auto, config.telemetry.format);
    assert_eq!("error", config.telemetry.filter);

    let mut both = tree;
    both.insert(String::from(FORMAT_ENV), String::from("pretty"));
    both.insert(String::from(FILTER_ENV), String::from("debug,hyper=warn"));
    let config = Config::from_sources(Some(file), &both)?;
    assert_eq!(Format::Pretty, config.telemetry.format);
    assert_eq!("debug,hyper=warn", config.telemetry.filter);

    let config = Config::from_sources(None, &env(FORMAT_ENV, "json"))?;
    assert_eq!(Format::Json, config.telemetry.format);
    assert_eq!(DEFAULT_FILTER, config.telemetry.filter);
    Ok(())
}

#[test]
fn a_log_format_outside_the_three_is_refused_naming_the_key() {
    let error = Config::from_sources(None, &env(FORMAT_ENV, "xml"))
        .expect_err("xml is not a console format");
    assert!(matches!(error, Error::Parse { .. }), "{error:?}");
    assert!(
        format!("{:?}", StdError::source(&error)).contains("format"),
        "the refusal names the key: {error:?}"
    );
}

#[test]
fn a_log_format_outside_the_three_exits_seventy_eight() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
        .arg("serve")
        .env_remove("FERROBRIDGE_CONFIG")
        .env(FORMAT_ENV, "xml")
        .output()
        .expect("the binary runs");
    assert_eq!(
        Some(i32::from(ferrobridge_server::EXIT_CONFIG)),
        output.status.code(),
        "a refused format is EX_CONFIG"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("format"),
        "the refusal names the key: {stderr}"
    );
    assert!(output.stdout.is_empty(), "no banner before a refused start");
}
