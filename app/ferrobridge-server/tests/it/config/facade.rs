// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[facade]` section: off until present, its policy, its store, and
//! its refusals.

use ferrobridge_server::config::{Config, Error};
use std::collections::BTreeMap;
use std::error::Error as StdError;

#[test]
fn the_facade_is_off_until_its_section_turns_it_on() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(None, &BTreeMap::new())?.resolve()?;
    assert!(
        settings.facade.is_none(),
        "the facade lane is off until [facade] enabled is set"
    );
    assert!(settings.mapping_directory.is_none());
    Ok(())
}

#[test]
fn an_enabled_facade_resolves_its_policy_and_its_store() -> Result<(), Box<dyn StdError>> {
    let text = concat!(
        "[mappings]\ndirectory = \"/srv/mappings\"\ntemplates = \"/srv/templates\"\n\n",
        "[facade]\nenabled = true\n",
        "base_url = \"https://bridge.invalid/fhir\"\n",
        "ehr_policy = \"create_on_first_write\"\n",
        "identity_store = \"/var/lib/ferrobridge/identity.redb\"\n",
        "composition_language = \"en\"\ncomposition_territory = \"GB\"\n",
    );

    let settings = Config::from_sources(Some(text), &BTreeMap::new())?.resolve()?;
    let facade = settings.facade.ok_or("the facade lane is on")?;
    assert_eq!("https://bridge.invalid/fhir", facade.settings.base_url);
    assert_eq!(
        ferrobridge_server::facade::ehr::Policy::CreateOnFirstWrite,
        facade.settings.ehr_policy
    );
    assert_eq!("en", facade.settings.language);
    assert_eq!("GB", facade.settings.territory);
    assert_eq!(
        std::path::Path::new("/var/lib/ferrobridge/identity.redb"),
        facade.identity_store
    );
    assert_eq!(
        Some(std::path::Path::new("/srv/mappings")),
        settings.mapping_directory.as_deref()
    );
    Ok(())
}

#[test]
fn an_ehr_policy_the_facade_does_not_know_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let text = concat!(
        "[facade]\nenabled = true\nehr_policy = \"whatever\"\n",
        "composition_language = \"en\"\ncomposition_territory = \"GB\"\n",
    );

    let error = Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("the policy is one of two values");
    match &error {
        Error::EhrPolicy { key, value } => {
            assert_eq!("facade.ehr_policy", key);
            assert_eq!("whatever", value);
        }
        other => return Err(format!("expected an ehr policy error, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn an_enabled_facade_without_a_composition_language_refuses_to_boot()
-> Result<(), Box<dyn StdError>> {
    let text = "[facade]\nenabled = true\ncomposition_territory = \"GB\"\n";

    let error = Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("a clinical record has no default language");
    match &error {
        Error::Missing { key } => assert_eq!("facade.composition_language", key),
        other => return Err(format!("expected a missing key, got {other:?}").into()),
    }
    Ok(())
}
