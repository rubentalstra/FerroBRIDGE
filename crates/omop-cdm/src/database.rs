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
use std::collections::BTreeSet;
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

/// A step of [`init`] and [`constrain`], in the order it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InitStep {
    /// Opening the transaction the whole step runs in.
    Begin,
    /// Reading which CDM tables the schema already holds, and `CDM_SOURCE`.
    Survey,
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
            Self::Survey => "reading the schema's tables",
            Self::Schema => "creating the schema",
            Self::Tables => "the table DDL",
            Self::PrimaryKeys => "the primary keys",
            Self::Indices => "the indices",
            Self::Commit => "committing the transaction",
        })
    }
}

/// What [`init`] found in the schema, and so what it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Init {
    /// The schema held none of the CDM tables, and every table, primary key
    /// and index was created.
    Created,
    /// The schema already held every CDM table, and nothing was applied.
    AlreadyInitialised {
        /// The distinct `cdm_version` values of the `CDM_SOURCE` rows, in
        /// order; empty when `CDM_SOURCE` holds no row.
        cdm_versions: Vec<String>,
    },
}

/// [`init`] refused the schema, or PostgreSQL refused a step of it; nothing
/// of the initialization is left behind.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InitError {
    /// PostgreSQL refused a step.
    #[error("PostgreSQL refused {step} while initializing CDM schema {schema}")]
    Refused {
        /// The schema being initialized.
        schema: SchemaName,
        /// The step PostgreSQL refused.
        step: InitStep,
        /// What PostgreSQL reported.
        #[source]
        source: sqlx::Error,
    },
    /// The schema holds some of the CDM tables and not the others.
    #[error(
        "CDM schema {schema} holds some of the CDM v5.4 tables and lacks {count}: {list}",
        count = missing.len(),
        list = missing.join(", ")
    )]
    Partial {
        /// The schema being initialized.
        schema: SchemaName,
        /// The CDM tables the schema lacks, in the order the metadata names
        /// them.
        missing: Vec<&'static str>,
    },
}

impl InitError {
    /// Returns the schema being initialized.
    #[must_use]
    pub fn schema(&self) -> &SchemaName {
        match self {
            Self::Refused { schema, .. } | Self::Partial { schema, .. } => schema,
        }
    }

    /// Returns the step PostgreSQL refused, when it refused one.
    #[must_use]
    pub fn step(&self) -> Option<InitStep> {
        match self {
            Self::Refused { step, .. } => Some(*step),
            Self::Partial { .. } => None,
        }
    }
}

/// Creates the CDM v5.4 tables, primary keys and indices in the pool's
/// schema, in the order OHDSI's rendered files come in, unless the schema
/// already holds them.
///
/// The schema's tables are read first. A schema that holds none of the CDM
/// tables is created, with the schema itself when it is absent; one that holds
/// every CDM table is left as it is and reported with the `cdm_version` of its
/// `CDM_SOURCE` rows; one that holds some is refused, naming the rest.
/// Everything runs in one transaction, so a refusal leaves the database as it
/// was. The vendored DDL text is applied as OHDSI renders it.
///
/// The foreign keys of `OMOPCDM_postgresql_5.4_constraints.sql` are a
/// separate step, [`constrain`].
///
/// # Errors
///
/// Returns [`InitError::Partial`] for a schema that holds some of the CDM
/// tables, and [`InitError::Refused`] naming the refused [`InitStep`], with
/// the database's own error as its source.
pub async fn init(pool: &CdmPool) -> Result<Init, InitError> {
    let schema = pool.schema();
    let fail = |step| {
        move |source| InitError::Refused {
            schema: schema.clone(),
            step,
            source,
        }
    };
    let mut transaction = pool.pool().begin().await.map_err(fail(InitStep::Begin))?;
    let present: BTreeSet<String> = sqlx::query_scalar!(
        r#"SELECT tablename::text AS "tablename!" FROM pg_catalog.pg_tables WHERE schemaname = $1"#,
        schema.as_str()
    )
    .fetch_all(&mut *transaction)
    .await
    .map_err(fail(InitStep::Survey))?
    .into_iter()
    .collect();
    let missing: Vec<&'static str> = generated::TABLES
        .iter()
        .map(|table| table.name)
        .filter(|name| !present.contains(*name))
        .collect();
    if missing.is_empty() {
        let cdm_versions = sqlx::query_scalar!(
            r#"SELECT DISTINCT cdm_version AS "cdm_version!" FROM cdm_source
               WHERE cdm_version IS NOT NULL ORDER BY 1"#
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(fail(InitStep::Survey))?;
        transaction.commit().await.map_err(fail(InitStep::Commit))?;
        return Ok(Init::AlreadyInitialised { cdm_versions });
    }
    if missing.len() < generated::TABLES.len() {
        return Err(InitError::Partial {
            schema: schema.clone(),
            missing,
        });
    }
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
    ];
    for (step, statements) in steps {
        // The text is OHDSI's vendored DDL and a `SchemaName`, which admits
        // only `[a-z_][a-z0-9_]*`, so nothing a caller writes reaches it as SQL.
        sqlx::raw_sql(sqlx::AssertSqlSafe(statements))
            .execute(&mut *transaction)
            .await
            .map_err(fail(step))?;
    }
    transaction.commit().await.map_err(fail(InitStep::Commit))?;
    Ok(Init::Created)
}

/// The OHDSI file [`constrain`] applies.
pub const CONSTRAINTS_FILE: &str = "OMOPCDM_postgresql_5.4_constraints.sql";

/// PostgreSQL refused a statement of [`constrain`]; none of the constraints
/// are left behind.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConstraintError {
    /// PostgreSQL refused to open or commit the transaction.
    #[error("PostgreSQL refused {step} while constraining CDM schema {schema}")]
    Transaction {
        /// The schema being constrained.
        schema: SchemaName,
        /// [`InitStep::Begin`] or [`InitStep::Commit`].
        step: InitStep,
        /// What PostgreSQL reported.
        #[source]
        source: sqlx::Error,
    },
    /// PostgreSQL refused one statement of the file.
    #[error(
        "PostgreSQL refused line {line} of {CONSTRAINTS_FILE} in CDM schema {schema}: {statement}"
    )]
    Statement {
        /// The schema being constrained.
        schema: SchemaName,
        /// The line of the vendored file the statement starts on, from 1.
        line: usize,
        /// The statement as the vendored file spells it.
        statement: String,
        /// What PostgreSQL reported.
        #[source]
        source: sqlx::Error,
    },
}

/// Applies the foreign keys of `OMOPCDM_postgresql_5.4_constraints.sql` to
/// the pool's schema, one statement at a time in the file's order, and
/// returns how many were applied.
///
/// Everything runs in one transaction, so a refusal leaves the schema
/// without any of them. OHDSI's file at tag `v5.4.3` declares foreign keys
/// onto `vocabulary (vocabulary_id)`, which its primary keys file gives no
/// key, so PostgreSQL refuses it there and the refusal names that statement.
///
/// # Errors
///
/// Returns [`ConstraintError::Statement`] naming the line and the text of
/// the refused statement, and [`ConstraintError::Transaction`] when the
/// transaction cannot be opened or committed.
pub async fn constrain(pool: &CdmPool) -> Result<usize, ConstraintError> {
    let schema = pool.schema();
    let transaction_failed = |step| {
        move |source| ConstraintError::Transaction {
            schema: schema.clone(),
            step,
            source,
        }
    };
    let mut transaction = pool
        .pool()
        .begin()
        .await
        .map_err(transaction_failed(InitStep::Begin))?;
    let statements = statements(generated::ddl::CONSTRAINTS);
    for (line, statement) in &statements {
        // As in `init`: vendored text and a checked `SchemaName` only.
        let sql = ddl::in_schema(statement, schema);
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *transaction)
            .await
            .map_err(|source| ConstraintError::Statement {
                schema: schema.clone(),
                line: *line,
                statement: statement.clone(),
                source,
            })?;
    }
    transaction
        .commit()
        .await
        .map_err(transaction_failed(InitStep::Commit))?;
    Ok(statements.len())
}

/// Returns the statements of `sql` with the line each starts on, from 1.
///
/// OHDSI's rendered files end every statement with `;` at the end of a line
/// and carry only `--` line comments, which are dropped.
fn statements(sql: &str) -> Vec<(usize, String)> {
    let mut statements = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (index, text) in sql.lines().enumerate() {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        let (_, statement) =
            current.get_or_insert_with(|| (index.saturating_add(1), String::new()));
        if !statement.is_empty() {
            statement.push('\n');
        }
        statement.push_str(trimmed);
        if trimmed.ends_with(';') {
            statements.extend(current.take());
        }
    }
    statements.extend(current);
    statements
}

#[cfg(test)]
mod tests {
    use super::statements;
    use crate::generated;

    #[test]
    fn every_constraint_statement_is_found_on_its_vendored_line() {
        let found = statements(generated::ddl::CONSTRAINTS);
        let lines: Vec<&str> = generated::ddl::CONSTRAINTS.lines().collect();
        assert_eq!(178, found.len(), "the v5.4.3 file carries 178 statements");
        for (line, statement) in &found {
            assert_eq!(
                Some(&statement.as_str()),
                lines.get(line - 1),
                "the statement does not sit on line {line}"
            );
        }
    }

    #[test]
    fn a_statement_over_several_lines_starts_on_its_first() {
        let found = statements("-- header\n\nALTER TABLE a\n  ADD x;\nALTER TABLE b ADD y;\n");
        assert_eq!(
            vec![
                (3, String::from("ALTER TABLE a\nADD x;")),
                (5, String::from("ALTER TABLE b ADD y;")),
            ],
            found
        );
    }
}
