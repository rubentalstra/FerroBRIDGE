// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The configuration contract: the file, the environment over it, the `_file`
//! secrets, and every refusal.

use ferrobridge_server::config::{Config, Error};
use ferrobridge_server::telemetry::{DEFAULT_FILTER, FILTER_ENV, FORMAT_ENV, Format};
use secrecy::ExposeSecret;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::io::Write;
use std::time::Duration;

/// A file with every section set, so a test can override one key at a time.
const FULL: &str = r#"
[server]
listen = "0.0.0.0:9000"
request_timeout_ms = 1000
shutdown_timeout_ms = 2000
body_limit_bytes = 4096

[telemetry]
format = "json"
filter = "debug"
logged_query_parameters = ["_count"]

[cdr]
base_url = "http://cdr.invalid/v1"
timeout_ms = 5000

[cdr.retry]
max_attempts = 5
initial_backoff_ms = 10
max_backoff_ms = 20

[terminology]
base_url = "http://tx.invalid/r4"
wire_version = "r4b"

[cdm]
url = "postgres://bridge@db.invalid/cdm?sslmode=disable"

[mappings]
directory = "/srv/ferrobridge/mappings"
templates = "/srv/ferrobridge/templates"

[operations]
enabled = false
device_reference = "Device/bridge-one"
composer = "A synthetic composer"
composition_language = "de"
composition_territory = "DE"
"#;

/// Returns the environment map a single override makes.
fn env(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

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

#[test]
fn a_secret_file_sibling_is_read_at_boot() -> Result<(), Box<dyn StdError>> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "synthetic-token")?;
    let text = format!(
        "[cdr]\nbase_url = \"http://cdr.invalid/v1\"\n\n[cdr.credentials]\nbearer_token_file = {:?}\n",
        file.path()
    );

    let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
    let cdr = settings.cdr.as_ref().ok_or("the CDR lane is on")?;
    match cdr.credentials.as_ref() {
        Some(openehr_its::rest::client::Credentials::Bearer(token)) => {
            assert_eq!(
                "synthetic-token",
                token.expose_secret(),
                "the trailing newline of the file is not part of the secret"
            );
        }
        other => return Err(format!("expected a bearer token, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_cdm_url_file_sibling_is_read_at_boot() -> Result<(), Box<dyn StdError>> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "postgres://bridge@db.invalid/cdm?sslmode=disable")?;
    let text = format!("[cdm]\nurl_file = {:?}\n", file.path());

    let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
    assert_eq!(
        Some("postgres://bridge@db.invalid/cdm?sslmode=disable"),
        settings.cdm.as_ref().map(|cdm| cdm.url.expose_secret())
    );
    Ok(())
}

#[test]
fn a_cdm_tls_ca_file_is_read_through_its_environment_name() -> Result<(), Box<dyn StdError>> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "not a certificate")?;
    let environment = BTreeMap::from([(
        String::from("FERROBRIDGE__CDM__TLS_CA_FILE"),
        file.path().display().to_string(),
    )]);
    let text = "[cdm]\nurl = \"postgres://bridge@db.invalid/cdm?sslmode=verify-full\"\n";

    let error = Config::from_sources(Some(text), &environment)?
        .resolve()
        .expect_err("the file holds no certificate");
    assert!(
        matches!(&error, Error::CdmConnection { key, .. } if key == "cdm.tls_ca_file"),
        "the refusal names the key the variable sets: {error:?}"
    );
    Ok(())
}

#[test]
fn a_value_and_its_file_sibling_together_refuse_to_boot() -> Result<(), Box<dyn StdError>> {
    let text = concat!(
        "[cdr]\nbase_url = \"http://cdr.invalid/v1\"\n\n",
        "[cdr.credentials]\nbearer_token = \"inline\"\nbearer_token_file = \"/run/secrets/token\"\n",
    );

    let error = Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("a secret set twice is a boot error");
    match &error {
        Error::Conflict { key } => assert_eq!("cdr.credentials.bearer_token", key),
        other => return Err(format!("expected a conflict, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_secret_file_that_cannot_be_read_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let text = concat!(
        "[cdr]\nbase_url = \"http://cdr.invalid/v1\"\n\n",
        "[cdr.credentials]\nbearer_token_file = \"/nonexistent/token\"\n",
    );

    let error = Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("an unreadable secret is a boot error");
    match &error {
        Error::Secret { key, .. } => {
            assert_eq!("cdr.credentials.bearer_token_file", key);
            assert!(
                StdError::source(&error).is_some(),
                "the refusal keeps the file system's own cause"
            );
        }
        other => return Err(format!("expected a secret error, got {other:?}").into()),
    }
    Ok(())
}

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

#[test]
fn a_credentials_section_naming_two_schemes_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let text = concat!(
        "[cdr]\nbase_url = \"http://cdr.invalid/v1\"\n\n",
        "[cdr.credentials]\nbearer_token = \"t\"\nuser = \"bridge\"\npassword = \"p\"\n",
    );

    let error = Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("one scheme at a time");
    match &error {
        Error::Scheme { section } => assert_eq!("cdr.credentials", section),
        other => return Err(format!("expected a scheme error, got {other:?}").into()),
    }
    Ok(())
}

#[test]
fn a_user_without_a_password_refuses_to_boot() -> Result<(), Box<dyn StdError>> {
    let text = concat!(
        "[cdr]\nbase_url = \"http://cdr.invalid/v1\"\n\n",
        "[cdr.credentials]\nuser = \"bridge\"\n",
    );

    let error = Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("basic authentication needs both halves");
    match &error {
        Error::Missing { key } => assert_eq!("cdr.credentials.password", key),
        other => return Err(format!("expected a missing key, got {other:?}").into()),
    }
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
