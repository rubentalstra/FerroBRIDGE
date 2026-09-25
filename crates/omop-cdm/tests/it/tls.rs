// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Both CDM clients against a PostgreSQL that admits TCP connections over
//! TLS only, one `sslmode` per case, as libpq documents the modes
//! (<https://www.postgresql.org/docs/current/libpq-ssl.html>).
//!
//! The server refuses a plaintext connection, so a client that connects has
//! connected over TLS. The cases run only when `FERROBRIDGE_E2E=1` admits the
//! harness, and say they skipped when no `openssl` is on `PATH`.

use ferrobridge_testkit::containers::{self, Postgres};
use ferrobridge_testkit::tls::{TlsError, TlsMaterial};
use omop_cdm::connection::CdmConnection;
use omop_cdm::database::CdmPool;
use omop_cdm::ddl::SchemaName;
use omop_cdm::writer::{CdmWriter, PersonPolicy, WriteError};
use sqlx::postgres::PgPoolOptions;
use std::error::Error;
use std::time::Duration;

/// Starts the TLS-only server, or returns `None` when the case cannot run.
#[expect(
    clippy::print_stderr,
    reason = "a case that cannot generate its certificates says it skipped"
)]
async fn server() -> Result<Option<(Postgres, TlsMaterial)>, Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(None);
    }
    let material = match TlsMaterial::generate() {
        Ok(material) => material,
        Err(TlsError::MissingOpenssl) => {
            eprintln!("skipped: the TLS cases generate their certificates with openssl");
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    let postgres = containers::postgres_tls(&material).await?;
    Ok(Some((postgres, material)))
}

/// Opens a pool over `connection`.
async fn pool(connection: &CdmConnection) -> Result<CdmPool, Box<dyn Error>> {
    Ok(CdmPool::connect(
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10)),
        connection.pool_options(),
        SchemaName::new("cdm")?,
    )
    .await?)
}

/// Opens a writer over `connection`, returning the writer's own outcome.
async fn writer(
    connection: &CdmConnection,
) -> Result<Result<CdmWriter, WriteError>, Box<dyn Error>> {
    Ok(CdmWriter::connect_with(
        connection,
        SchemaName::new("cdm")?,
        SchemaName::new("ferrobridge")?,
        PersonPolicy::CreateOnFirstSight,
    )
    .await)
}

/// Asserts that both clients connect over `connection` and that the pool's
/// session is encrypted.
async fn both_connect(connection: &CdmConnection) -> Result<(), Box<dyn Error>> {
    let pool = pool(connection).await?;
    let encrypted: bool =
        sqlx::query_scalar("SELECT ssl FROM pg_catalog.pg_stat_ssl WHERE pid = pg_backend_pid()")
            .fetch_one(pool.pool())
            .await?;
    assert!(encrypted, "the pool's session is not encrypted");
    let mut writer = writer(connection).await??;
    writer.init().await?;
    Ok(())
}

/// Asserts that neither client connects over `connection`.
async fn neither_connects(connection: &CdmConnection) -> Result<(), Box<dyn Error>> {
    assert!(
        pool(connection).await.is_err(),
        "the pool connected with {connection:?}"
    );
    let Err(refusal) = writer(connection).await? else {
        return Err(format!("the writer connected with {connection:?}").into());
    };
    assert!(
        matches!(refusal, WriteError::Connect { .. }),
        "the writer's refusal is not a connection one: {refusal:?}"
    );
    Ok(())
}

#[tokio::test]
async fn require_without_a_ca_encrypts_both_clients() -> Result<(), Box<dyn Error>> {
    let Some((postgres, _)) = server().await? else {
        return Ok(());
    };
    let url = format!("{}?sslmode=require", postgres.url());
    both_connect(&CdmConnection::new(&url, None)?).await
}

#[tokio::test]
async fn verify_full_with_the_ca_connects_both_clients() -> Result<(), Box<dyn Error>> {
    let Some((postgres, material)) = server().await? else {
        return Ok(());
    };
    let url = format!("{}?sslmode=verify-full", postgres.url());
    both_connect(&CdmConnection::new(&url, Some(material.ca()))?).await
}

#[tokio::test]
async fn verify_ca_with_the_ca_connects_both_clients() -> Result<(), Box<dyn Error>> {
    let Some((postgres, material)) = server().await? else {
        return Ok(());
    };
    let url = format!("{}?sslmode=verify-ca", postgres.url());
    both_connect(&CdmConnection::new(&url, Some(material.ca()))?).await
}

#[tokio::test]
async fn verify_full_refuses_a_certificate_the_ca_did_not_sign() -> Result<(), Box<dyn Error>> {
    let Some((postgres, material)) = server().await? else {
        return Ok(());
    };
    let url = format!("{}?sslmode=verify-full", postgres.url());
    neither_connects(&CdmConnection::new(&url, Some(material.stranger_ca()))?).await?;
    neither_connects(&CdmConnection::new(&url, None)?).await
}

#[tokio::test]
async fn require_with_a_ca_checks_the_chain() -> Result<(), Box<dyn Error>> {
    let Some((postgres, material)) = server().await? else {
        return Ok(());
    };
    let url = format!("{}?sslmode=require", postgres.url());
    neither_connects(&CdmConnection::new(&url, Some(material.stranger_ca()))?).await
}

#[tokio::test]
async fn disable_is_refused_by_a_server_that_admits_tls_only() -> Result<(), Box<dyn Error>> {
    let Some((postgres, _)) = server().await? else {
        return Ok(());
    };
    let url = format!("{}?sslmode=disable", postgres.url());
    neither_connects(&CdmConnection::new(&url, None)?).await
}
