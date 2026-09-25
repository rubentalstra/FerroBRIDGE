// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `database::init` on the three states a schema can be in, and
//! `database::constrain` over the vendored constraints file.
//!
//! The container-backed tests run only when `FERROBRIDGE_E2E=1` admits the
//! harness. No specification governs what `init` does on an initialised
//! schema: our own design.

use ferrobridge_testkit::containers::{self, Postgres};
use omop_cdm::database::{self, CdmPool, ConstraintError, Init, InitError};
use omop_cdm::ddl::SchemaName;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::error::Error;

/// The schema the tests build the CDM in.
const SCHEMA: &str = "cdm";

/// Opens a pool on `postgres` bound to [`SCHEMA`].
async fn connect(postgres: &Postgres) -> Result<CdmPool, Box<dyn Error>> {
    let options: PgConnectOptions = postgres.url().parse()?;
    Ok(CdmPool::connect(
        PgPoolOptions::new().max_connections(2),
        options,
        SchemaName::new(SCHEMA)?,
    )
    .await?)
}

/// Returns how many tables the schema holds.
async fn tables(pool: &CdmPool) -> Result<i64, Box<dyn Error>> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_tables WHERE schemaname = $1")
            .bind(SCHEMA)
            .fetch_one(pool.pool())
            .await?,
    )
}

/// Returns how many foreign keys the schema holds.
async fn foreign_keys(pool: &CdmPool) -> Result<i64, Box<dyn Error>> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_constraint c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.connamespace \
         WHERE n.nspname = $1 AND c.contype = 'f'",
    )
    .bind(SCHEMA)
    .fetch_one(pool.pool())
    .await?)
}

#[tokio::test]
async fn a_fresh_schema_is_created_with_every_table() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    assert_eq!(Init::Created, database::init(&pool).await?);
    assert_eq!(
        i64::try_from(omop_cdm::generated::TABLES.len())?,
        tables(&pool).await?,
        "init created a different number of tables than the model has"
    );
    Ok(())
}

#[tokio::test]
async fn a_schema_holding_only_other_tables_is_created() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    sqlx::raw_sql("CREATE SCHEMA cdm; CREATE TABLE cdm.site_notes (id integer)")
        .execute(pool.pool())
        .await?;
    assert_eq!(Init::Created, database::init(&pool).await?);
    Ok(())
}

#[tokio::test]
async fn a_second_init_reports_the_initialised_schema_and_applies_nothing()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    database::init(&pool).await?;
    assert_eq!(
        Init::AlreadyInitialised {
            cdm_versions: Vec::new()
        },
        database::init(&pool).await?,
        "an empty CDM_SOURCE carries no version"
    );
    Ok(())
}

#[tokio::test]
async fn an_initialised_schema_reports_the_version_cdm_source_records() -> Result<(), Box<dyn Error>>
{
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    database::init(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO cdm.cdm_source (cdm_source_name, cdm_source_abbreviation, cdm_holder, \
         source_release_date, cdm_release_date, cdm_version, cdm_version_concept_id, \
         vocabulary_version) VALUES ('Synthetic', 'SYN', 'Tests', '2026-09-01', '2026-09-01', \
         'v5.4', 756265, 'v5.0 01-SEP-26')",
    )
    .execute(pool.pool())
    .await?;
    assert_eq!(
        Init::AlreadyInitialised {
            cdm_versions: vec![String::from("v5.4")]
        },
        database::init(&pool).await?
    );
    Ok(())
}

#[tokio::test]
async fn a_partial_schema_is_refused_naming_the_missing_tables() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    database::init(&pool).await?;
    sqlx::raw_sql("DROP TABLE cdm.note_nlp; DROP TABLE cdm.cohort")
        .execute(pool.pool())
        .await?;
    let refusal = database::init(&pool)
        .await
        .expect_err("two CDM tables are missing");
    let InitError::Partial { schema, missing } = &refusal else {
        panic!("the refusal is not the partial one: {refusal:?}");
    };
    assert_eq!(SCHEMA, schema.as_str());
    assert_eq!(&vec!["note_nlp", "cohort"], missing, "the missing tables");
    let message = refusal.to_string();
    assert!(
        message.contains("note_nlp") && message.contains("cohort"),
        "{message}"
    );
    assert_eq!(
        i64::try_from(omop_cdm::generated::TABLES.len() - 2)?,
        tables(&pool).await?,
        "the refusal changed the schema"
    );
    Ok(())
}

#[tokio::test]
async fn init_leaves_the_foreign_keys_out() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    database::init(&pool).await?;
    assert_eq!(0, foreign_keys(&pool).await?);
    Ok(())
}

/// Pins the refusal of `OMOPCDM_postgresql_5.4_constraints.sql` at v5.4.3:
/// line 157 declares a foreign key onto `vocabulary (vocabulary_id)`, which
/// the primary keys file gives no key, so PostgreSQL answers SQLSTATE 42830
/// (<https://www.postgresql.org/docs/18/errcodes-appendix.html>).
#[tokio::test]
async fn constrain_reports_the_refused_statement_and_leaves_nothing() -> Result<(), Box<dyn Error>>
{
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let postgres = containers::postgres().await?;
    let pool = connect(&postgres).await?;
    database::init(&pool).await?;
    let refusal = database::constrain(&pool)
        .await
        .expect_err("the vocabulary foreign key has no key to reference");
    let ConstraintError::Statement {
        line,
        statement,
        source,
        ..
    } = &refusal
    else {
        panic!("the refusal is not a statement one: {refusal:?}");
    };
    assert_eq!(157, *line, "the refused line");
    assert!(
        statement.contains("fpk_concept_vocabulary_id"),
        "the refused statement: {statement}"
    );
    let code = source
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .ok_or("the refusal carries no SQLSTATE")?;
    assert_eq!("42830", code, "the SQLSTATE");
    assert!(
        refusal.to_string().contains(statement.as_str()),
        "the message carries the statement verbatim: {refusal}"
    );
    assert_eq!(
        0,
        foreign_keys(&pool).await?,
        "a foreign key was left behind"
    );
    Ok(())
}
