// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDM database: a connection pool bound to one schema, and the embedded
//! DDL applied to it.
//!
//! OHDSI's DDL leaves the schema to the deployment, and the queries of
//! [`crate::vocabulary`] are checked at compile time against fixed text, so
//! they name their tables unqualified. [`CdmPool`] makes that safe: every
//! connection it opens carries `search_path` set to the one schema, so an
//! unqualified `concept` can only be that schema's table.
//!
//! No specification governs this: our own design.

use crate::ddl::{self, SchemaName};
use crate::generated;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use std::fmt;

/// A connection pool whose every connection resolves unqualified table names
/// in one CDM schema.
#[derive(Debug, Clone)]
pub struct CdmPool {
    pool: PgPool,
    schema: SchemaName,
}

/// The pool could not reach the database.
#[derive(Debug, thiserror::Error)]
#[error("cannot connect to the CDM database for schema {schema}")]
pub struct ConnectError {
    schema: SchemaName,
    #[source]
    source: sqlx::Error,
}

impl ConnectError {
    /// Returns the schema the pool was being opened for.
    #[must_use]
    pub fn schema(&self) -> &SchemaName {
        &self.schema
    }
}

impl CdmPool {
    /// Opens a pool on `options` whose connections search only `schema`.
    ///
    /// `pool` carries the caller's sizing and timeouts. The pool opens one
    /// connection before it returns, so an unreachable database or a refused
    /// login is reported here.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectError`] with the database's own error as its source
    /// when the first connection cannot be opened.
    pub async fn connect(
        pool: PgPoolOptions,
        options: PgConnectOptions,
        schema: SchemaName,
    ) -> Result<Self, ConnectError> {
        // NOTE: PostgreSQL docs, 5.10.3 "The Schema Search Path": an unqualified
        // name resolves against search_path, and pg_catalog is searched first.
        let options = options.options([("search_path", schema.as_str())]);
        match pool.connect_with(options).await {
            Ok(pool) => Ok(Self { pool, schema }),
            Err(source) => Err(ConnectError { schema, source }),
        }
    }

    /// Returns the pool.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Returns the schema every connection searches.
    #[must_use]
    pub fn schema(&self) -> &SchemaName {
        &self.schema
    }
}

/// A step of [`init`], in the order it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InitStep {
    /// Opening the transaction the whole initialization runs in.
    Begin,
    /// Creating the schema when it is absent.
    Schema,
    /// `OMOPCDM_postgresql_5.4_ddl.sql`, the tables.
    Tables,
    /// `OMOPCDM_postgresql_5.4_primary_keys.sql`.
    PrimaryKeys,
    /// `OMOPCDM_postgresql_5.4_indices.sql`.
    Indices,
    /// Committing the transaction.
    Commit,
}

impl fmt::Display for InitStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Begin => "opening the transaction",
            Self::Schema => "creating the schema",
            Self::Tables => "the table DDL",
            Self::PrimaryKeys => "the primary keys",
            Self::Indices => "the indices",
            Self::Commit => "committing the transaction",
        })
    }
}

/// PostgreSQL refused a step of [`init`]; nothing of the initialization is
/// left behind.
#[derive(Debug, thiserror::Error)]
#[error("PostgreSQL refused {step} while initializing CDM schema {schema}")]
pub struct InitError {
    schema: SchemaName,
    step: InitStep,
    #[source]
    source: sqlx::Error,
}

impl InitError {
    /// Returns the schema being initialized.
    #[must_use]
    pub fn schema(&self) -> &SchemaName {
        &self.schema
    }

    /// Returns the step PostgreSQL refused.
    #[must_use]
    pub fn step(&self) -> InitStep {
        self.step
    }
}

/// Creates the CDM v5.4 tables, primary keys and indices in the pool's
/// schema, in the order OHDSI's rendered files come in.
///
/// The schema is created when it is absent. Everything runs in one
/// transaction, so a refusal leaves the database as it was.
///
/// OHDSI's `OMOPCDM_postgresql_5.4_constraints.sql` is not applied: at tag
/// `v5.4.3` it declares foreign keys onto `vocabulary (vocabulary_id)`, which
/// its `primary_keys.sql` gives no key, and PostgreSQL refuses the file.
///
/// # Errors
///
/// Returns [`InitError`] naming the refused [`InitStep`], with the database's
/// own error as its source.
pub async fn init(pool: &CdmPool) -> Result<(), InitError> {
    let schema = pool.schema();
    let fail = |step| {
        move |source| InitError {
            schema: schema.clone(),
            step,
            source,
        }
    };
    let mut transaction = pool.pool().begin().await.map_err(fail(InitStep::Begin))?;
    let steps = [
        (
            InitStep::Schema,
            format!("CREATE SCHEMA IF NOT EXISTS {schema}"),
        ),
        (
            InitStep::Tables,
            ddl::in_schema(generated::ddl::DDL, schema),
        ),
        (
            InitStep::PrimaryKeys,
            ddl::in_schema(generated::ddl::PRIMARY_KEYS, schema),
        ),
        (
            InitStep::Indices,
            ddl::in_schema(generated::ddl::INDICES, schema),
        ),
        // TODO(#232): the foreign keys of OMOPCDM_postgresql_5.4_constraints.sql,
        // which PostgreSQL refuses at v5.4.3 for want of a vocabulary key.
    ];
    for (step, statements) in steps {
        // The text is OHDSI's vendored DDL and a `SchemaName`, which admits
        // only `[a-z_][a-z0-9_]*`, so nothing a caller writes reaches it as SQL.
        sqlx::raw_sql(sqlx::AssertSqlSafe(statements))
            .execute(&mut *transaction)
            .await
            .map_err(fail(step))?;
    }
    transaction.commit().await.map_err(fail(InitStep::Commit))
}
