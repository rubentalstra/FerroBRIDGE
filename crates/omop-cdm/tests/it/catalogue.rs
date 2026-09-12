// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The embedded DDL applied to a real PostgreSQL, read back out of
//! `information_schema` and compared with the generated column metadata.
//!
//! Both tests run only when `FERROBRIDGE_E2E=1` admits the container harness.

use ferrobridge_testkit::containers;
use std::collections::BTreeMap;
use std::error::Error;
use tokio::task::JoinHandle;
use tokio_postgres::Client;

/// The task that drives one PostgreSQL connection.
type Driver = JoinHandle<Result<(), tokio_postgres::Error>>;

/// The schema the DDL is pointed at.
const SCHEMA: &str = "cdm";

#[tokio::test]
async fn the_embedded_ddl_builds_the_generated_catalogue() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let (client, driver) = connect(postgres.url()).await?;

    client
        .batch_execute(&format!("CREATE SCHEMA {SCHEMA}"))
        .await?;
    for statements in [
        omop_cdm::generated::ddl::DDL,
        omop_cdm::generated::ddl::PRIMARY_KEYS,
        omop_cdm::generated::ddl::INDICES,
    ] {
        client
            .batch_execute(&omop_cdm::ddl::with_schema(statements, SCHEMA)?)
            .await?;
    }

    let catalogue = columns_of(&client).await?;
    for table in &omop_cdm::generated::TABLES {
        let columns = catalogue
            .get(table.name)
            .ok_or_else(|| format!("the applied DDL created no {} table", table.name))?;
        let generated: Vec<&str> = table.columns.iter().map(|column| column.name).collect();
        assert_eq!(
            &generated, columns,
            "the {} columns of the catalogue and of the generated metadata disagree",
            table.name
        );
    }
    assert_eq!(
        omop_cdm::generated::TABLES.len(),
        catalogue.len(),
        "the applied DDL created a different number of tables than the model has"
    );

    driver.abort();
    Ok(())
}

/// Pins what OHDSI's rendered constraint file does to an empty database.
///
/// `OMOPCDM_postgresql_5.4_primary_keys.sql` at v5.4.3 declares no primary key
/// for `vocabulary`, while `OMOPCDM_postgresql_5.4_constraints.sql` declares
/// foreign keys onto `vocabulary (vocabulary_id)`, so PostgreSQL refuses the
/// file with SQLSTATE 42830
/// (<https://www.postgresql.org/docs/18/errcodes-appendix.html>).
#[tokio::test]
async fn the_vendored_constraints_refuse_the_vocabulary_foreign_key() -> Result<(), Box<dyn Error>>
{
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let (client, driver) = connect(postgres.url()).await?;

    client
        .batch_execute(&format!("CREATE SCHEMA {SCHEMA}"))
        .await?;
    for statements in [
        omop_cdm::generated::ddl::DDL,
        omop_cdm::generated::ddl::PRIMARY_KEYS,
    ] {
        client
            .batch_execute(&omop_cdm::ddl::with_schema(statements, SCHEMA)?)
            .await?;
    }

    let refusal = client
        .batch_execute(&omop_cdm::ddl::with_schema(
            omop_cdm::generated::ddl::CONSTRAINTS,
            SCHEMA,
        )?)
        .await
        .expect_err("the vocabulary foreign key has no key to reference");
    assert_eq!(
        Some(&tokio_postgres::error::SqlState::INVALID_FOREIGN_KEY),
        refusal.code(),
        "the refusal is not the missing-unique-constraint one: {refusal}"
    );
    let message = refusal
        .as_db_error()
        .map(tokio_postgres::error::DbError::message)
        .ok_or("the refusal carries no database error")?;
    assert!(
        message.contains("vocabulary"),
        "the refusal does not name the vocabulary table: {message}"
    );

    driver.abort();
    Ok(())
}

/// Connects to `url` and returns the client with the task driving it.
///
/// The driver task is the connection future itself, which resolves when the
/// connection ends; a connection that dies early is reported by the next
/// client call, which the caller propagates.
async fn connect(url: &str) -> Result<(Client, Driver), Box<dyn Error>> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    Ok((client, tokio::spawn(connection)))
}

/// Returns the columns of every table of the schema, in catalogue order.
async fn columns_of(client: &Client) -> Result<BTreeMap<String, Vec<String>>, Box<dyn Error>> {
    let rows = client
        .query(
            "SELECT table_name, column_name FROM information_schema.columns \
             WHERE table_schema = $1 ORDER BY table_name, ordinal_position",
            &[&SCHEMA],
        )
        .await?;
    let mut catalogue: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &rows {
        let table: String = row.try_get("table_name")?;
        let column: String = row.try_get("column_name")?;
        catalogue.entry(table).or_default().push(column);
    }
    Ok(catalogue)
}
