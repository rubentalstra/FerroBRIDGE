// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[hl7v2]` section: off until present, every key, and every refusal.

use ferrobridge_server::config::{Config, Error};
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use super::env;
use super::refusal;

/// A facade the HL7 v2 face can write through, with `face` as its section.
fn with_face(face: &str) -> String {
    format!(
        "[facade]\nenabled = true\ncomposition_language = \"en\"\ncomposition_territory = \"GB\"\n\n[hl7v2]\nenabled = true\n{face}"
    )
}

#[test]
fn the_hl7v2_face_is_off_until_its_section_turns_it_on() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(None, &BTreeMap::new())?.resolve()?;
    assert!(
        settings.hl7v2.is_none(),
        "no [hl7v2] enabled leaves the face off"
    );
    Ok(())
}

#[test]
fn an_enabled_hl7v2_face_resolves_every_key() -> Result<(), Box<dyn StdError>> {
    let text = with_face(concat!(
        "listen = \"0.0.0.0:2576\"\n",
        "default_charset = \"8859/1\"\n",
        "concept_maps = \"/srv/v2mappings/package\"\n",
        "supplements = [\"/srv/v2mappings/local\"]\n",
        "unmapped_entries = \"refuse\"\n",
        "ehr_policy = \"create_on_first_write\"\n",
        "idle_timeout_ms = 0\n",
        "frame_timeout_ms = 5000\n",
        "frame_limit_bytes = 65536\n",
        "log_outcomes = true\n",
        "senders = [{ namespace_id = \"NORTHLAB\" }, { universal_id = \"1.2.3.4\", universal_id_type = \"ISO\" }]\n",
        "profiles = [{ resource_type = \"Observation\", profile = \"http://example.org/fhir/StructureDefinition/lab\" }]\n",
    ));
    let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
    let face = settings.hl7v2.ok_or("the face is on")?;
    assert_eq!("0.0.0.0:2576", face.listen.to_string());
    assert_eq!(
        ferrobridge_hl7v2::decode::Charset::Iso8859(1),
        face.default_charset
    );
    assert_eq!(
        std::path::Path::new("/srv/v2mappings/package"),
        face.concept_maps
    );
    assert_eq!(
        vec![std::path::PathBuf::from("/srv/v2mappings/local")],
        face.supplements
    );
    assert_eq!(
        ferrobridge_server::facade::ingest::UnmappedEntries::Refuse,
        face.unmapped
    );
    assert_eq!(
        Some(ferrobridge_server::facade::ehr::Policy::CreateOnFirstWrite),
        face.ehr_policy
    );
    assert_eq!(
        None, face.timeouts.idle,
        "0 never closes an idle connection"
    );
    assert_eq!(Some(Duration::from_secs(5)), face.timeouts.frame);
    assert_eq!(65536, face.frame_limit);
    assert!(face.log_outcomes);
    assert_eq!(
        vec![
            ferrobridge_server::hl7v2::Sender::Namespace(String::from("NORTHLAB")),
            ferrobridge_server::hl7v2::Sender::Universal {
                id: String::from("1.2.3.4"),
                kind: String::from("ISO"),
            },
        ],
        face.senders
    );
    assert_eq!(
        Some("http://example.org/fhir/StructureDefinition/lab"),
        face.profiles.get("Observation").map(String::as_str)
    );
    Ok(())
}

#[test]
fn the_hl7v2_defaults_skip_and_count_with_both_timeouts() -> Result<(), Box<dyn StdError>> {
    let text = with_face("concept_maps = \"/srv/v2mappings/package\"\n");
    let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
    let face = settings.hl7v2.ok_or("the face is on")?;
    assert_eq!("127.0.0.1:2575", face.listen.to_string());
    assert_eq!(
        ferrobridge_hl7v2::decode::Charset::Ascii,
        face.default_charset
    );
    assert_eq!(
        ferrobridge_server::facade::ingest::UnmappedEntries::SkipAndCount,
        face.unmapped
    );
    assert_eq!(None, face.ehr_policy, "the facade's policy stands");
    assert_eq!(Some(Duration::from_millis(300_000)), face.timeouts.idle);
    assert_eq!(Some(Duration::from_secs(30)), face.timeouts.frame);
    assert_eq!(1024 * 1024, face.frame_limit);
    assert!(!face.log_outcomes);
    assert!(face.senders.is_empty(), "an empty list accepts any sender");
    Ok(())
}

#[test]
fn an_hl7v2_key_is_set_through_its_environment_name() -> Result<(), Box<dyn StdError>> {
    let text = with_face("concept_maps = \"/srv/v2mappings/package\"\n");
    let settings = Config::from_sources(
        Some(&text),
        &env("FERROBRIDGE__HL7V2__LISTEN", "0.0.0.0:2577"),
    )?
    .resolve()?;
    let face = settings.hl7v2.ok_or("the face is on")?;
    assert_eq!("0.0.0.0:2577", face.listen.to_string());
    Ok(())
}

#[test]
fn an_hl7v2_face_without_the_facade_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    match &refusal("[hl7v2]\nenabled = true\nconcept_maps = \"/srv/p\"\n")? {
        Error::Needs { key, needs } => {
            assert_eq!("hl7v2.enabled", key);
            assert_eq!("facade.enabled", needs);
        }
        other => return Err(format!("expected a needs error, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn an_hl7v2_face_without_its_concept_maps_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    match &refusal(&with_face(""))? {
        Error::Missing { key } => assert_eq!("hl7v2.concept_maps", key),
        other => return Err(format!("expected a missing key, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn every_bad_hl7v2_value_refuses_to_boot_naming_its_key() -> Result<(), Box<dyn StdError>> {
    let maps = "concept_maps = \"/srv/p\"\n";
    let cases = [
        ("listen = \"nowhere\"\n", "hl7v2.listen"),
        ("default_charset = \"EBCDIC\"\n", "hl7v2.default_charset"),
        ("unmapped_entries = \"drop\"\n", "hl7v2.unmapped_entries"),
        ("ehr_policy = \"always\"\n", "hl7v2.ehr_policy"),
        ("frame_limit_bytes = 0\n", "hl7v2.frame_limit_bytes"),
        (
            "profiles = [{ resource_type = \"Observatoin\", profile = \"http://example.org/p\" }]\n",
            "hl7v2.profiles.resource_type",
        ),
        (
            "profiles = [{ resource_type = \"Observation\", profile = \"not a url\" }]\n",
            "hl7v2.profiles.profile",
        ),
        (
            "profiles = [{ resource_type = \"Observation\", profile = \"http://example.org/a\" }, { resource_type = \"Observation\", profile = \"http://example.org/b\" }]\n",
            "hl7v2.profiles",
        ),
        (
            "senders = [{ namespace_id = \"A\", universal_id = \"1.2\", universal_id_type = \"ISO\" }]\n",
            "hl7v2.senders",
        ),
        ("senders = [{ universal_id = \"1.2\" }]\n", "hl7v2.senders"),
        ("senders = [{}]\n", "hl7v2.senders"),
    ];
    for (line, expected) in cases {
        let error = refusal(&with_face(&format!("{maps}{line}")))?;
        let key = match &error {
            Error::Listen { key, .. }
            | Error::Charset { key, .. }
            | Error::UnmappedEntries { key, .. }
            | Error::EhrPolicy { key, .. }
            | Error::FrameLimit { key }
            | Error::ResourceType { key, .. }
            | Error::Url { key, .. }
            | Error::Duplicate { key, .. }
            | Error::Sender { key } => key.as_str(),
            other => return Err(format!("{line}: unexpected refusal {other:?}").into()),
        };
        assert_eq!(expected, key, "{line}");
    }
    Ok(())
}

#[test]
fn an_unknown_hl7v2_key_refuses_to_boot() {
    let error = Config::from_sources(
        Some(&with_face("sending_facilities = []\n")),
        &BTreeMap::new(),
    )
    .expect_err("an unknown key is refused");
    assert!(matches!(error, Error::Parse { .. }), "{error:?}");
}
