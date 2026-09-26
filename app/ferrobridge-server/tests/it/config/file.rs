// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The file: every section read, and the lanes it turns on.

use ferrobridge_server::config::{Config, Error};
use secrecy::ExposeSecret;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use super::FULL;

#[test]
fn a_file_states_every_section_and_the_resolver_reads_it() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(Some(FULL), &BTreeMap::new())?.resolve()?;
    assert_eq!("0.0.0.0:9000", settings.server.listen.to_string());
    assert_eq!(Duration::from_millis(1000), settings.server.request_timeout);
    assert_eq!(
        Duration::from_millis(2000),
        settings.server.shutdown_timeout
    );
    assert_eq!(4096, settings.server.body_limit);
    assert_eq!(
        vec![String::from("_count")],
        settings.telemetry.logged_query_parameters
    );
    let cdr = settings.cdr.as_ref().ok_or("the CDR lane is on")?;
    assert_eq!("http://cdr.invalid/v1", cdr.base_url.as_str());
    assert_eq!(5, cdr.retry.max_attempts);
    assert_eq!(Duration::from_millis(10), cdr.retry.initial_backoff);
    let terminology = settings.terminology.as_ref().ok_or("the lane is on")?;
    assert_eq!(
        ferrobridge_term::config::WireVersion::R4B,
        terminology.wire_version
    );
    assert_eq!(
        Some("postgres://bridge@db.invalid/cdm?sslmode=disable"),
        settings.cdm.as_ref().map(|cdm| cdm.url.expose_secret())
    );
    let mappings = settings.mappings.as_ref().ok_or("the mapping set is on")?;
    assert_eq!(
        std::path::Path::new("/srv/ferrobridge/mappings"),
        mappings.directory
    );
    assert_eq!(
        std::path::Path::new("/srv/ferrobridge/templates"),
        mappings.templates
    );
    assert!(!settings.operations.enabled);
    assert_eq!("Device/bridge-one", settings.operations.device_reference);
    assert_eq!("A synthetic composer", settings.operations.composer);
    assert_eq!(
        Some("de"),
        settings.operations.composition_language.as_deref()
    );
    assert_eq!(
        Some("DE"),
        settings.operations.composition_territory.as_deref()
    );
    Ok(())
}

#[test]
fn the_operations_lane_is_on_by_default_and_names_its_own_device() -> Result<(), Box<dyn StdError>>
{
    let settings = Config::from_sources(None, &BTreeMap::new())?.resolve()?;
    assert!(settings.operations.enabled);
    assert!(
        settings.mappings.is_none(),
        "no [mappings] section leaves the lane with nothing to serve"
    );
    assert_eq!(
        format!("Device/ferrobridge-{}", env!("CARGO_PKG_VERSION")),
        settings.operations.device_reference
    );
    Ok(())
}

#[test]
fn a_mappings_section_with_no_directory_is_refused() -> Result<(), Box<dyn StdError>> {
    let file = "[mappings]\ntemplates = \"/srv/templates\"\n";
    let error = Config::from_sources(Some(file), &BTreeMap::new())?
        .resolve()
        .err()
        .ok_or("a section present with no directory is refused")?;
    assert!(
        matches!(error, Error::Missing { ref key } if key == "mappings.directory"),
        "the refusal names the key: {error}"
    );
    Ok(())
}

#[test]
fn a_mapping_directory_with_neither_templates_nor_a_cdr_is_refused_at_resolve()
-> Result<(), Box<dyn StdError>> {
    let file = "[mappings]\ndirectory = \"/srv/mappings\"\n";
    let error = Config::from_sources(Some(file), &BTreeMap::new())?
        .resolve()
        .err()
        .ok_or("the operations lane has no template source")?;
    let Error::Mappings { ref source } = error else {
        panic!("the refusal is a mapping-set refusal: {error}");
    };
    assert!(
        matches!(
            **source,
            ferrobridge_server::mappings::Error::NoTemplateSource { ref directory }
                if directory == std::path::Path::new("/srv/mappings")
        ),
        "{source}"
    );
    assert_eq!("the mapping set could not be loaded", error.to_string());
    let rendered = source.to_string();
    assert!(
        rendered.contains("[mappings] templates") && rendered.contains("[cdr]"),
        "the refusal names both sources: {rendered}"
    );
    Ok(())
}

#[test]
fn the_omocl_directory_is_read_from_the_mappings_section() -> Result<(), Box<dyn StdError>> {
    let text = "[mappings]\nomocl = \"/srv/omocl\"\n";
    let settings = Config::from_sources(Some(text), &BTreeMap::new())?.resolve()?;
    assert_eq!(
        Some(std::path::Path::new("/srv/omocl")),
        settings.omocl_directory.as_deref()
    );
    assert!(
        settings.mapping_directory.is_none(),
        "the FHIRconnect tree stays unset"
    );
    Ok(())
}
