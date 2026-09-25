// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `cdm init` through the binary, and the `[cdm]` TLS settings the two CDM
//! clients connect with.
//!
//! The `sslmode` values are libpq's
//! (<https://www.postgresql.org/docs/current/libpq-ssl.html>); the modes that
//! fall back to plaintext are refused when the configuration is read. The
//! container-backed cases run only when `FERROBRIDGE_E2E=1` admits the
//! harness. No specification governs `cdm init`: our own design.

use ferrobridge_server::config::{Config, Error};
use ferrobridge_testkit::containers::{self, Postgres};
use ferrobridge_testkit::tls::{TlsError, TlsMaterial};
use omop_cdm::connection::SslMode;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::io::Write;
use std::process::Output;

/// Resolves `text`, returning the refusal.
fn refusal(text: &str) -> Result<Error, Box<dyn StdError>> {
    match Config::from_sources(Some(text), &BTreeMap::new())?.resolve() {
        Ok(_) => Err("the configuration resolved".into()),
        Err(error) => Ok(error),
    }
}

/// Returns `error` and every cause behind it as one line.
fn chain(error: &dyn StdError) -> String {
    let mut line = error.to_string();
    let mut cause = error.source();
    while let Some(source) = cause {
        line.push_str(": ");
        line.push_str(&source.to_string());
        cause = source.source();
    }
    line
}

/// Generates the TLS material, or returns `None` when no `openssl` is there.
#[expect(
    clippy::print_stderr,
    reason = "a case that cannot generate its certificates says it skipped"
)]
fn material() -> Result<Option<TlsMaterial>, Box<dyn StdError>> {
    match TlsMaterial::generate() {
        Ok(material) => Ok(Some(material)),
        Err(TlsError::MissingOpenssl) => {
            eprintln!("skipped: the TLS cases generate their certificates with openssl");
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[test]
fn a_fallback_sslmode_is_refused_naming_the_mode() -> Result<(), Box<dyn StdError>> {
    for mode in ["prefer", "allow"] {
        let error = refusal(&format!(
            "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode={mode}\"\n"
        ))?;
        assert!(
            matches!(&error, Error::CdmConnection { key, .. } if key == "cdm.url"),
            "{error:?}"
        );
        let line = chain(&error);
        assert!(line.contains(&format!("sslmode={mode}")), "{line}");
    }
    Ok(())
}

#[test]
fn an_unknown_sslmode_is_refused_naming_the_mode() -> Result<(), Box<dyn StdError>> {
    let error = refusal("[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=strict\"\n")?;
    assert!(chain(&error).contains("sslmode=strict"), "{error:?}");
    Ok(())
}

#[test]
fn a_fallback_sslmode_exits_seventy_eight() -> Result<(), Box<dyn StdError>> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(
        file,
        "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=prefer\""
    )?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
        .arg("--config")
        .arg(file.path())
        .args(["cdm", "init"])
        .env_remove("FERROBRIDGE_CONFIG")
        .output()?;
    assert_eq!(
        Some(i32::from(ferrobridge_server::EXIT_CONFIG)),
        output.status.code(),
        "a refused sslmode is EX_CONFIG"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("sslmode=prefer"), "{stderr}");
    Ok(())
}

#[test]
fn every_libpq_mode_that_never_falls_back_resolves() -> Result<(), Box<dyn StdError>> {
    for mode in [
        SslMode::Disable,
        SslMode::Require,
        SslMode::VerifyCa,
        SslMode::VerifyFull,
    ] {
        let text = format!("[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode={mode}\"\n");
        let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
        let cdm = settings.cdm.ok_or("the CDM lane is on")?;
        assert_eq!(mode, cdm.connection.ssl_mode());
    }
    Ok(())
}

#[test]
fn a_url_without_sslmode_is_refused_naming_the_four_modes() -> Result<(), Box<dyn StdError>> {
    for url in [
        "postgres://db.invalid/cdm",
        "postgres://db.invalid/cdm?application_name=bridge",
    ] {
        let error = refusal(&format!("[cdm]\nurl = \"{url}\"\n"))?;
        assert!(
            matches!(&error, Error::CdmConnection { key, .. } if key == "cdm.url"),
            "{error:?}"
        );
        let line = chain(&error);
        assert!(
            line.contains("disable, require, verify-ca or verify-full"),
            "{line}"
        );
    }
    Ok(())
}

#[test]
fn a_url_without_sslmode_exits_seventy_eight() -> Result<(), Box<dyn StdError>> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "[cdm]\nurl = \"postgres://db.invalid/cdm\"")?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
        .arg("--config")
        .arg(file.path())
        .args(["cdm", "init"])
        .env_remove("FERROBRIDGE_CONFIG")
        .output()?;
    assert_eq!(
        Some(i32::from(ferrobridge_server::EXIT_CONFIG)),
        output.status.code()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("names no sslmode"), "{stderr}");
    Ok(())
}

#[test]
fn a_libpq_tls_variable_in_the_environment_is_refused_naming_it() -> Result<(), Box<dyn StdError>> {
    for name in ["PGSSLROOTCERT", "PGSSLCERT", "PGSSLKEY"] {
        let mut file = tempfile::NamedTempFile::new()?;
        writeln!(
            file,
            "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=verify-full\""
        )?;
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
            .arg("--config")
            .arg(file.path())
            .args(["cdm", "init"])
            .env_remove("FERROBRIDGE_CONFIG")
            .env(name, "/nonexistent/file.pem")
            .output()?;
        assert_eq!(
            Some(i32::from(ferrobridge_server::EXIT_CONFIG)),
            output.status.code(),
            "{name}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(name), "{stderr}");
    }
    Ok(())
}

/// A deployment with the ETL lane on over a TLS-checked CDM database whose URL
/// carries a password.
const ETL_OVER_TLS: &str = r#"
[cdm]
url = "postgres://bridge:s3cret@cdm.invalid:5433/omop?sslmode=verify-full"

[etl]
aql = "SELECT e/ehr_id/value AS ehr_id, vo/uid/value AS versioned_object_uid, v/uid/value AS version_uid, c AS composition FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY v/uid/value"
type_concept_id = 32817
observation_period_type_concept_id = 32880
"#;

#[test]
fn the_etl_lane_reports_the_sslmode_and_no_credential() -> Result<(), Box<dyn StdError>> {
    let settings = Config::from_sources(Some(ETL_OVER_TLS), &BTreeMap::new())?.resolve()?;
    let lanes = ferrobridge_server::startup::lanes(&settings);
    let etl = lanes
        .iter()
        .find(|lane| lane.name == "etl")
        .ok_or("the etl lane")?;
    assert_eq!(Some("verify-full"), etl.cdm_sslmode);
    assert_eq!(Some("cdm.invalid:5433"), etl.cdm_host.as_deref());
    assert!(
        !format!("{etl:?}").contains("s3cret"),
        "no credential in the lane: {etl:?}"
    );
    let facade = lanes
        .iter()
        .find(|lane| lane.name == "facade")
        .ok_or("the facade lane")?;
    assert_eq!(
        None, facade.cdm_sslmode,
        "only the ETL lane reaches the CDM"
    );
    Ok(())
}

#[test]
fn a_ca_inline_and_in_a_file_together_are_refused() -> Result<(), Box<dyn StdError>> {
    let error = refusal(
        "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=verify-full\"\n\
         tls_ca = \"x\"\ntls_ca_file = \"/nonexistent\"\n",
    )?;
    assert!(
        matches!(&error, Error::Conflict { key } if key == "cdm.tls_ca"),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn a_ca_file_that_holds_no_certificate_is_refused_naming_the_key() -> Result<(), Box<dyn StdError>>
{
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "not a certificate")?;
    let error = refusal(&format!(
        "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=verify-full\"\ntls_ca_file = {:?}\n",
        file.path()
    ))?;
    assert!(
        matches!(&error, Error::CdmConnection { key, .. } if key == "cdm.tls_ca_file"),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn an_unreadable_ca_file_is_refused_naming_the_key() -> Result<(), Box<dyn StdError>> {
    let error = refusal(
        "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=verify-full\"\n\
         tls_ca_file = \"/nonexistent/ca.pem\"\n",
    )?;
    assert!(
        matches!(&error, Error::Secret { key, .. } if key == "cdm.tls_ca_file"),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn a_ca_with_sslmode_disable_is_refused() -> Result<(), Box<dyn StdError>> {
    let error =
        refusal("[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=disable\"\ntls_ca = \"x\"\n")?;
    assert!(
        matches!(&error, Error::CdmConnection { key, .. } if key == "cdm.tls_ca"),
        "{error:?}"
    );
    Ok(())
}

#[test]
fn a_ca_file_is_read_at_configuration() -> Result<(), Box<dyn StdError>> {
    let Some(material) = material()? else {
        return Ok(());
    };
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(material.ca())?;
    let text = format!(
        "[cdm]\nurl = \"postgres://db.invalid/cdm?sslmode=verify-full\"\ntls_ca_file = {:?}\n",
        file.path()
    );
    let settings = Config::from_sources(Some(&text), &BTreeMap::new())?.resolve()?;
    let cdm = settings.cdm.ok_or("the CDM lane is on")?;
    assert!(cdm.connection.has_ca(), "the CA was not read");
    Ok(())
}

/// Runs `ferrobridge cdm init` with `extra` over a configuration naming
/// `url` and `lines`.
fn cdm_init(url: &str, lines: &str, extra: &[&str]) -> Result<Output, Box<dyn StdError>> {
    let mut file = tempfile::NamedTempFile::new()?;
    writeln!(file, "[cdm]\nurl = {url:?}\n{lines}")?;
    Ok(
        std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
            .arg("--config")
            .arg(file.path())
            .args(["cdm", "init"])
            .args(extra)
            .env_remove("FERROBRIDGE_CONFIG")
            .output()?,
    )
}

/// Returns what the run wrote, both streams.
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Runs `statements` on `postgres` over a connection of its own.
async fn execute(postgres: &Postgres, statements: &str) -> Result<(), Box<dyn StdError>> {
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    client.batch_execute(statements).await?;
    driver.abort();
    Ok(())
}

/// Returns how many foreign keys schema `cdm` holds.
async fn foreign_keys(postgres: &Postgres) -> Result<i64, Box<dyn StdError>> {
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    let row = client
        .query_one(
            "SELECT count(*) FROM pg_catalog.pg_constraint c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.connamespace \
             WHERE n.nspname = 'cdm' AND c.contype = 'f'",
            &[],
        )
        .await?;
    driver.abort();
    Ok(row.try_get(0)?)
}

#[tokio::test]
async fn a_second_cdm_init_exits_zero_and_says_the_schema_is_initialised()
-> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let first = cdm_init(postgres.url(), "", &[])?;
    assert!(first.status.success(), "{}", text(&first));
    assert!(
        text(&first).contains("created the CDM 5.4 tables"),
        "{}",
        text(&first)
    );
    let second = cdm_init(postgres.url(), "", &[])?;
    assert!(second.status.success(), "{}", text(&second));
    assert!(
        text(&second).contains("already initialised"),
        "{}",
        text(&second)
    );
    Ok(())
}

#[tokio::test]
async fn cdm_init_on_a_partial_schema_fails_naming_the_missing_tables()
-> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    execute(
        &postgres,
        "CREATE SCHEMA cdm; CREATE TABLE cdm.person (person_id integer)",
    )
    .await?;
    let output = cdm_init(postgres.url(), "", &[])?;
    assert_eq!(Some(1), output.status.code(), "{}", text(&output));
    let written = text(&output);
    assert!(
        written.contains("observation_period") && written.contains("cdm_source"),
        "the refusal names the missing tables: {written}"
    );
    Ok(())
}

#[tokio::test]
async fn cdm_init_without_the_flag_leaves_the_foreign_keys_out() -> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let output = cdm_init(postgres.url(), "", &[])?;
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(0, foreign_keys(&postgres).await?);
    Ok(())
}

/// At OHDSI tag v5.4.3 PostgreSQL refuses line 157 of the constraints file
/// with SQLSTATE 42830, so the flag fails the run and names the statement.
#[tokio::test]
async fn cdm_init_with_constraints_reports_the_refused_statement() -> Result<(), Box<dyn StdError>>
{
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let output = cdm_init(postgres.url(), "", &["--with-constraints"])?;
    assert_eq!(Some(1), output.status.code(), "{}", text(&output));
    let written = text(&output);
    assert!(written.contains("line 157"), "{written}");
    assert!(written.contains("fpk_concept_vocabulary_id"), "{written}");
    assert!(written.contains("created the CDM 5.4 tables"), "{written}");
    assert_eq!(0, foreign_keys(&postgres).await?, "a foreign key was left");
    let again = cdm_init(postgres.url(), "", &[])?;
    assert!(
        again.status.success() && text(&again).contains("already initialised"),
        "the tables stay after the refused constraints: {}",
        text(&again)
    );
    Ok(())
}

#[tokio::test]
async fn cdm_init_connects_over_verify_full_with_the_ca_file() -> Result<(), Box<dyn StdError>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let Some(material) = material()? else {
        return Ok(());
    };
    let postgres = containers::postgres_tls(&material).await?;
    let mut ca = tempfile::NamedTempFile::new()?;
    ca.write_all(material.ca())?;
    let output = cdm_init(
        &format!("{}?sslmode=verify-full", postgres.url()),
        &format!("tls_ca_file = {:?}", ca.path()),
        &[],
    )?;
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("bridge schema ferrobridge: in place"),
        "{}",
        text(&output)
    );
    Ok(())
}
