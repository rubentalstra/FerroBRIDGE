// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals to boot on an unknown key, a wrong type or a bad value.

use ferrobridge_server::config::{Config, Error};
use std::collections::BTreeMap;
use std::error::Error as StdError;

#[test]
fn an_unknown_key_refuses_to_boot_and_the_message_names_it() -> Result<(), Box<dyn StdError>> {
    let error = Config::from_sources(Some("[server]\nlisten_port = 8080\n"), &BTreeMap::new())
        .expect_err("an unknown key is a boot error");
    let Error::Parse { source } = &error else {
        return Err(format!("expected a parse error, got {error:?}").into());
    };
    assert!(
        source.to_string().contains("listen_port"),
        "the refusal names the key: {source}"
    );
    Ok(())
}

#[test]
fn an_unknown_section_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = Config::from_sources(Some("[cache]\nsize = 1\n"), &BTreeMap::new())
        .expect_err("an unknown section is a boot error");
    let Error::Parse { source } = &error else {
        return Err(format!("expected a parse error, got {error:?}").into());
    };
    assert!(source.to_string().contains("cache"), "{source}");
    Ok(())
}

#[test]
fn a_value_of_the_wrong_type_refuses_to_boot_and_the_message_names_its_key()
-> Result<(), Box<dyn StdError>> {
    let error = Config::from_sources(
        Some("[server]\nrequest_timeout_ms = \"thirty seconds\"\n"),
        &BTreeMap::new(),
    )
    .expect_err("a string is not a millisecond count");
    let Error::Parse { source } = &error else {
        return Err(format!("expected a parse error, got {error:?}").into());
    };
    assert!(
        source.to_string().contains("request_timeout_ms"),
        "the refusal names the key: {source}"
    );
    Ok(())
}

#[test]
fn an_unknown_log_format_refuses_to_boot_and_names_its_key() -> Result<(), Box<dyn StdError>> {
    let error = Config::from_sources(Some("[telemetry]\nformat = \"xml\"\n"), &BTreeMap::new())
        .expect_err("xml is not a rendering");
    let Error::Parse { source } = &error else {
        return Err(format!("expected a parse error, got {error:?}").into());
    };
    let rendered = source.to_string();
    assert!(rendered.contains("format"), "{rendered}");
    assert!(rendered.contains("xml"), "{rendered}");
    Ok(())
}

#[test]
fn a_bad_listen_address_a_bad_url_and_a_bad_release_each_name_their_key()
-> Result<(), Box<dyn StdError>> {
    let listen = Config::from_sources(Some("[server]\nlisten = \"nowhere\"\n"), &BTreeMap::new())?
        .resolve()
        .expect_err("nowhere is not a socket address");
    match &listen {
        Error::Listen { key, .. } => assert_eq!("server.listen", key),
        other => return Err(format!("expected a listen error, got {other:?}").into()),
    }

    let url = Config::from_sources(Some("[cdr]\nbase_url = \"not a url\"\n"), &BTreeMap::new())?
        .resolve()
        .expect_err("that is not a URL");
    match &url {
        Error::Url { key, .. } => assert_eq!("cdr.base_url", key),
        other => return Err(format!("expected a URL error, got {other:?}").into()),
    }

    let release = Config::from_sources(
        Some("[terminology]\nbase_url = \"http://tx.invalid/r5\"\nwire_version = \"r5\"\n"),
        &BTreeMap::new(),
    )?
    .resolve()
    .expect_err("this client does not speak R5");
    match &release {
        Error::WireVersion { key, value } => {
            assert_eq!("terminology.wire_version", key);
            assert_eq!("r5", value);
        }
        other => return Err(format!("expected a release error, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_section_that_is_present_and_names_no_url_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let error = Config::from_sources(Some("[cdr]\ntimeout_ms = 1000\n"), &BTreeMap::new())?
        .resolve()
        .expect_err("a CDR lane with no base URL cannot start");
    match &error {
        Error::Missing { key } => assert_eq!("cdr.base_url", key),
        other => return Err(format!("expected a missing key, got {other:?}").into()),
    }
    Ok(())
}
