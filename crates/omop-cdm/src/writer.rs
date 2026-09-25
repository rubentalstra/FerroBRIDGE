// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDM writer: one composition's record graph per transaction, through
//! binary `COPY`, with the natural-key side table that makes a re-run replace
//! its rows.
//!
//! Every CDM v5.4 primary key is a 32-bit `integer`, so the writer assigns
//! ids from one PostgreSQL sequence per table and records, in a schema of its
//! own beside the CDM, which natural key holds which id: the [`RecordKey`]
//! of each row, the `ehr_id` of each person, the key of each visit, the links
//! of each composition, and a watermark naming the version of each composition
//! last committed. [`CdmWriter::commit`] deletes the rows an earlier version
//! of the composition wrote, writes the new ones under the same ids, and
//! advances the watermark, all in one transaction, so a failure leaves the
//! database as it was.
//!
//! The rows cross into PostgreSQL through binary `COPY` into a temporary
//! staging table whose `integer` columns are binary and whose other columns
//! are text; one `INSERT ... SELECT` then casts dates, datetimes and floats
//! from their text, so this crate does no calendar or decimal arithmetic
//! (<https://www.postgresql.org/docs/16/sql-copy.html>).
//!
//! No specification governs any of this: our own design (the CDM leaves keys
//! and provenance to the ETL, <https://ohdsi.github.io/CommonDataModel/cdm54.html>).

use crate::ddl::SchemaName;
use crate::graph::{
    self, Cell, EhrId, EmptyIdentifier, RecordGraph, RecordKey, Reference, Report, Value,
    VersionUid, VersionedObjectUid, Visit, VisitKey,
};
use crate::meta::{ColumnMeta, TableMeta};
use std::collections::BTreeMap;
use std::fmt;
use tokio_postgres::binary_copy::BinaryCopyInWriter;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, NoTls, Transaction};

/// The identifier of one ETL run, recorded with every watermark it writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

impl RunId {
    /// Wraps a run identifier, refusing an empty one.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyIdentifier`] when `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, EmptyIdentifier> {
        let value = value.into();
        if value.is_empty() {
            return Err(EmptyIdentifier::of("run id"));
        }
        Ok(Self(value))
    }

    /// Returns the identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the writer does for an EHR it has no `PERSON` for.
///
/// One `ehr_id` is one person; reconciling a person across EHRs is the
/// deployment's decision and never the writer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonPolicy {
    /// Assigns the EHR a `person_id` the first time a row refers to it.
    CreateOnFirstSight,
    /// Refuses a row that refers to an EHR no earlier `PERSON` row named.
    Existing,
}

/// The two concepts every derived visit carries, from configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisitConcepts {
    /// `visit_concept_id`.
    pub visit_concept_id: i32,
    /// `visit_type_concept_id`.
    pub visit_type_concept_id: i32,
}

/// The version of a composition last committed, and the run that did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark {
    version_uid: VersionUid,
    run_id: RunId,
}

impl Watermark {
    /// Returns the version committed.
    #[must_use]
    pub fn version_uid(&self) -> &VersionUid {
        &self.version_uid
    }

    /// Returns the run that committed it.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

/// What the writer was doing when PostgreSQL refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Step {
    /// Creating the bridge schema and its tables.
    Init,
    /// Opening a transaction.
    Begin,
    /// Taking the per-composition lock.
    Lock,
    /// Reading the side table.
    Read,
    /// Deleting the rows an earlier version wrote to a table.
    Delete(&'static str),
    /// Assigning an id from a table's sequence.
    Allocate(&'static str),
    /// Recording the natural keys.
    Record,
    /// Staging and copying the rows of a table.
    Copy(&'static str),
    /// Moving the staged rows of a table into the CDM.
    Insert(&'static str),
    /// Advancing the watermark.
    Watermark,
    /// Rebuilding a derived table.
    Derive(&'static str),
    /// Committing the transaction.
    Commit,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => f.write_str("creating the bridge schema"),
            Self::Begin => f.write_str("opening the transaction"),
            Self::Lock => f.write_str("locking the composition"),
            Self::Read => f.write_str("reading the side table"),
            Self::Delete(table) => write!(f, "deleting the earlier rows of `{table}`"),
            Self::Allocate(table) => write!(f, "assigning an id for `{table}`"),
            Self::Record => f.write_str("recording the natural keys"),
            Self::Copy(table) => write!(f, "copying the rows of `{table}`"),
            Self::Insert(table) => write!(f, "inserting the rows of `{table}`"),
            Self::Watermark => f.write_str("advancing the watermark"),
            Self::Derive(table) => write!(f, "deriving `{table}`"),
            Self::Commit => f.write_str("committing the transaction"),
        }
    }
}

/// A write the CDM database refused or the writer could not complete.
///
/// A refusal inside [`CdmWriter::commit`] rolls the whole composition back.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// The connection could not be opened.
    #[error("cannot connect to the CDM database")]
    Connect {
        /// What the client reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// PostgreSQL refused a statement.
    #[error("PostgreSQL refused {step}")]
    Database {
        /// What the writer was doing.
        step: Step,
        /// What PostgreSQL reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// The sequence of a table has no id left for a 32-bit key.
    #[error("the id sequence of `{table}` is exhausted")]
    SequenceExhausted {
        /// The table.
        table: &'static str,
        /// What PostgreSQL reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// A row refers to an EHR that has no `PERSON`, and the policy creates
    /// none.
    #[error("no PERSON is known for the EHR {ehr_id}, and the policy creates none")]
    UnknownPerson {
        /// The EHR.
        ehr_id: EhrId,
    },
    /// A row refers to a visit the visit derivation has not written.
    #[error("no visit is known for the EHR {} under the source {}", key.ehr_id(), key.source())]
    UnknownVisit {
        /// The visit's key.
        key: VisitKey,
    },
    /// A row refers to a row that neither the graph nor an earlier commit
    /// holds.
    #[error("the row {key} of `{table}` is not known")]
    UnknownRow {
        /// The table.
        table: &'static str,
        /// The key.
        key: RecordKey,
    },
    /// The CDM metadata has no table the writer names.
    #[error("the CDM metadata refused a table the writer names")]
    Metadata(#[from] graph::GraphError),
    /// The side table holds an empty identifier.
    #[error("the side table holds an empty identifier")]
    Identifier(#[from] EmptyIdentifier),
    /// The side table names something this writer never writes.
    #[error("the side table holds `{value}` for {what}, which this writer never writes")]
    SideTable {
        /// What the value should have been.
        what: &'static str,
        /// The value found.
        value: String,
    },
}

/// Returns the refusal of `step`, or the exhaustion of `table`'s sequence.
fn database(step: Step) -> impl FnOnce(tokio_postgres::Error) -> WriteError {
    move |source| match step {
        Step::Allocate(table)
            if source.code() == Some(&SqlState::SEQUENCE_GENERATOR_LIMIT_EXCEEDED) =>
        {
            WriteError::SequenceExhausted { table, source }
        }
        step => WriteError::Database { step, source },
    }
}

/// Returns `name` as a quoted PostgreSQL identifier.
///
/// Every name the writer quotes is a CDM table or column from the generated
/// metadata, so none carries a double quote; quoting keeps `note_nlp.offset`,
/// a reserved word, a column name.
fn quoted(name: &str) -> String {
    format!("\"{name}\"")
}

/// Returns the name of the sequence the ids of `table` come from.
fn sequence(bridge: &SchemaName, table: &str) -> String {
    format!("{bridge}.{}", quoted(&format!("{table}_id_seq")))
}

/// One staged column value: binary `integer`, or text PostgreSQL casts.
#[derive(Debug, Clone, PartialEq)]
enum Staged {
    /// An `integer` column.
    Integer(Option<i32>),
    /// Any other column, as text.
    Text(Option<String>),
}

impl Staged {
    /// Returns the value as a `COPY` field.
    fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Integer(value) => value,
            Self::Text(value) => value,
        }
    }
}

/// Returns whether `column` stages as a binary `integer`.
fn is_integer(column: &ColumnMeta) -> bool {
    column.cdm_datatype == "integer"
}

/// Returns the SQL that moves the staged `column` into the CDM column.
fn cast(column: &ColumnMeta) -> String {
    let name = quoted(column.name);
    match column.cdm_datatype {
        "date" => format!("{name}::date"),
        "datetime" => format!("{name}::timestamp"),
        "float" => format!("{name}::numeric"),
        _ => name,
    }
}

/// Returns the staged form of `value`.
fn staged(value: &Value) -> Staged {
    match value {
        Value::Integer(number) => Staged::Integer(Some(*number)),
        // NOTE: Rust documentation, `impl Display for f64`: the output is the
        // shortest decimal that round-trips, so PostgreSQL's numeric reads it exactly.
        Value::Float(number) => Staged::Text(Some(number.to_string())),
        Value::Text(text) => Staged::Text(Some(text.clone())),
        Value::Date(date) => Staged::Text(Some(date.as_str().to_owned())),
        Value::Datetime(datetime) => Staged::Text(Some(datetime.as_str().to_owned())),
    }
}

/// Returns one staged row of `table`, in column order, with every column
/// `values` does not name set to NULL.
fn stage_row(table: &TableMeta, mut values: BTreeMap<&'static str, Staged>) -> Vec<Staged> {
    table
        .columns
        .iter()
        .map(|column| {
            values.remove(column.name).unwrap_or(if is_integer(column) {
                Staged::Integer(None)
            } else {
                Staged::Text(None)
            })
        })
        .collect()
}

/// Copies `rows` of `table` into the CDM through a staging table.
async fn copy_rows(
    transaction: &Transaction<'_>,
    cdm: &SchemaName,
    table: &'static TableMeta,
    rows: &[Vec<Staged>],
) -> Result<u64, WriteError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let stage = quoted(&format!("ferrobridge_stage_{}", table.name));
    let definition = table
        .columns
        .iter()
        .map(|column| {
            let kind = if is_integer(column) {
                "integer"
            } else {
                "text"
            };
            format!("{} {kind}", quoted(column.name))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let names = table
        .columns
        .iter()
        .map(|column| quoted(column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let casts = table
        .columns
        .iter()
        .map(cast)
        .collect::<Vec<_>>()
        .join(", ");
    let types: Vec<Type> = table
        .columns
        .iter()
        .map(|column| {
            if is_integer(column) {
                Type::INT4
            } else {
                Type::TEXT
            }
        })
        .collect();
    let step = Step::Copy(table.name);
    transaction
        .batch_execute(&format!(
            "CREATE TEMP TABLE IF NOT EXISTS {stage} ({definition}) ON COMMIT DELETE ROWS"
        ))
        .await
        .map_err(database(step))?;
    let sink = transaction
        .copy_in(&format!(
            "COPY pg_temp.{stage} ({names}) FROM STDIN (FORMAT binary)"
        ))
        .await
        .map_err(database(step))?;
    let writer = BinaryCopyInWriter::new(sink, &types);
    let mut writer = std::pin::pin!(writer);
    for row in rows {
        let fields: Vec<&(dyn ToSql + Sync)> = row.iter().map(Staged::as_sql).collect();
        writer
            .as_mut()
            .write(&fields)
            .await
            .map_err(database(step))?;
    }
    writer.as_mut().finish().await.map_err(database(step))?;
    let inserted = transaction
        .execute(
            &format!(
                "INSERT INTO {cdm}.{} ({names}) SELECT {casts} FROM pg_temp.{stage}",
                quoted(table.name)
            ),
            &[],
        )
        .await
        .map_err(database(Step::Insert(table.name)))?;
    transaction
        .batch_execute(&format!("TRUNCATE pg_temp.{stage}"))
        .await
        .map_err(database(Step::Insert(table.name)))?;
    Ok(inserted)
}

/// Returns the next id of `table` from its sequence.
async fn allocate(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    table: &'static str,
) -> Result<i32, WriteError> {
    let row = transaction
        .query_one(
            &format!("SELECT nextval('{}')::integer", sequence(bridge, table)),
            &[],
        )
        .await
        .map_err(database(Step::Allocate(table)))?;
    row.try_get(0).map_err(database(Step::Allocate(table)))
}

/// Resolves `ehr_id` to its `person_id`, assigning one when `create` allows.
async fn person_id(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    ehr_id: &EhrId,
    create: bool,
) -> Result<i32, WriteError> {
    let found = transaction
        .query_opt(
            &format!("SELECT person_id FROM {bridge}.person_map WHERE ehr_id = $1"),
            &[&ehr_id.as_str()],
        )
        .await
        .map_err(database(Step::Read))?;
    if let Some(row) = found {
        return row.try_get(0).map_err(database(Step::Read));
    }
    if !create {
        return Err(WriteError::UnknownPerson {
            ehr_id: ehr_id.clone(),
        });
    }
    let id = allocate(transaction, bridge, "person").await?;
    transaction
        .execute(
            &format!("INSERT INTO {bridge}.person_map (ehr_id, person_id) VALUES ($1, $2)"),
            &[&ehr_id.as_str(), &id],
        )
        .await
        .map_err(database(Step::Record))?;
    Ok(id)
}

/// The tables and sequences of the bridge schema.
fn bridge_ddl(bridge: &SchemaName) -> String {
    let mut statements = vec![
        format!("CREATE SCHEMA IF NOT EXISTS {bridge}"),
        format!(
            "CREATE TABLE IF NOT EXISTS {bridge}.person_map (
                ehr_id text PRIMARY KEY,
                person_id integer NOT NULL UNIQUE)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {bridge}.record (
                versioned_object_uid text NOT NULL,
                archetype_root_path text NOT NULL,
                occurrence_path text NOT NULL,
                cdm_table text NOT NULL,
                ehr_id text NOT NULL,
                mapping text NOT NULL,
                surrogate_id integer NOT NULL,
                PRIMARY KEY (versioned_object_uid, archetype_root_path, occurrence_path, cdm_table),
                UNIQUE (cdm_table, surrogate_id))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {bridge}.link (
                versioned_object_uid text NOT NULL,
                domain_concept_id_1 integer NOT NULL,
                fact_id_1 integer NOT NULL,
                domain_concept_id_2 integer NOT NULL,
                fact_id_2 integer NOT NULL,
                relationship_concept_id integer NOT NULL)"
        ),
        format!(
            "CREATE INDEX IF NOT EXISTS link_versioned_object_uid
                ON {bridge}.link (versioned_object_uid)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {bridge}.visit (
                ehr_id text NOT NULL,
                visit_source text NOT NULL,
                visit_occurrence_id integer NOT NULL UNIQUE,
                PRIMARY KEY (ehr_id, visit_source))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {bridge}.watermark (
                versioned_object_uid text PRIMARY KEY,
                ehr_id text NOT NULL,
                version_uid text NOT NULL,
                run_id text NOT NULL,
                committed_at timestamptz NOT NULL)"
        ),
    ];
    for table in &crate::generated::TABLES {
        if graph::cdm_table(table.name).is_ok() && graph::primary_key(table).is_some() {
            statements.push(format!(
                "CREATE SEQUENCE IF NOT EXISTS {} AS integer",
                sequence(bridge, table.name)
            ));
        }
    }
    statements.join(";\n")
}

/// Writes record graphs, visits and the derived tables into one CDM schema.
///
/// The writer holds one `tokio-postgres` connection of its own, never a
/// connection of a pool another client uses, because a composition's
/// statements must share its transaction.
#[derive(Debug)]
pub struct CdmWriter {
    client: Client,
    connection: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
    cdm: SchemaName,
    bridge: SchemaName,
    policy: PersonPolicy,
}

impl Drop for CdmWriter {
    fn drop(&mut self) {
        self.connection.abort();
    }
}

impl CdmWriter {
    /// Connects to the PostgreSQL database `url` names, writing the CDM
    /// tables of schema `cdm` and keeping the side table in schema `bridge`.
    ///
    /// The connection is made without TLS; a URL whose `sslmode` requires TLS
    /// is refused by the client rather than downgraded.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::Connect`] when the database cannot be reached or
    /// refuses the login.
    pub async fn connect(
        url: &str,
        cdm: SchemaName,
        bridge: SchemaName,
        policy: PersonPolicy,
    ) -> Result<Self, WriteError> {
        let (client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .map_err(|source| WriteError::Connect { source })?;
        let connection = tokio::spawn(connection);
        Ok(Self {
            client,
            connection,
            cdm,
            bridge,
            policy,
        })
    }

    /// Returns the CDM schema.
    #[must_use]
    pub fn cdm_schema(&self) -> &SchemaName {
        &self.cdm
    }

    /// Returns the bridge schema.
    #[must_use]
    pub fn bridge_schema(&self) -> &SchemaName {
        &self.bridge
    }

    /// Returns whether the connection has closed, after which every call
    /// fails.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// Returns the client and the CDM schema, for the derivations.
    pub(crate) fn parts(&mut self) -> (&mut Client, &SchemaName) {
        (&mut self.client, &self.cdm)
    }

    /// Creates the bridge schema, its tables and one id sequence per CDM
    /// table, leaving whatever already exists.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::Database`] with [`Step::Init`] when PostgreSQL
    /// refuses a statement; nothing is left behind.
    pub async fn init(&mut self) -> Result<(), WriteError> {
        let transaction = self
            .client
            .transaction()
            .await
            .map_err(database(Step::Begin))?;
        transaction
            .batch_execute(&bridge_ddl(&self.bridge))
            .await
            .map_err(database(Step::Init))?;
        transaction.commit().await.map_err(database(Step::Commit))
    }

    /// Returns the watermark of the versioned composition, when one was
    /// committed.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::Database`] when the side table cannot be read,
    /// and [`WriteError::SideTable`] when it holds an empty identifier.
    pub async fn watermark(
        &self,
        versioned_object_uid: &VersionedObjectUid,
    ) -> Result<Option<Watermark>, WriteError> {
        let row = self
            .client
            .query_opt(
                &format!(
                    "SELECT version_uid, run_id FROM {}.watermark WHERE versioned_object_uid = $1",
                    self.bridge
                ),
                &[&versioned_object_uid.as_str()],
            )
            .await
            .map_err(database(Step::Read))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let version: String = row.try_get(0).map_err(database(Step::Read))?;
        let run: String = row.try_get(1).map_err(database(Step::Read))?;
        Ok(Some(Watermark {
            version_uid: VersionUid::new(version)?,
            run_id: RunId::new(run)?,
        }))
    }

    /// Commits the record graph of one composition version and returns its
    /// report, completed with what was written.
    ///
    /// In one transaction: the rows and links an earlier version of the
    /// composition wrote are deleted, every row gets the id its natural key
    /// held before or a new one from its table's sequence, references are
    /// resolved, the rows and both directions of every link are copied in,
    /// and the watermark names this version and `run`. Committing the same
    /// graph again writes the same rows under the same ids.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::SequenceExhausted`] when a table has no id left,
    /// [`WriteError::UnknownPerson`], [`WriteError::UnknownVisit`] and
    /// [`WriteError::UnknownRow`] for a reference that does not resolve,
    /// [`WriteError::SideTable`] for a side table this writer did not write,
    /// and [`WriteError::Database`] for any statement PostgreSQL refuses. In
    /// every case the transaction is rolled back and nothing of the
    /// composition is written.
    pub async fn commit(&mut self, graph: &RecordGraph, run: &RunId) -> Result<Report, WriteError> {
        let Self {
            client,
            cdm,
            bridge,
            policy,
            ..
        } = self;
        let transaction = client.transaction().await.map_err(database(Step::Begin))?;
        let source = graph.source();
        let versioned_object_uid = source.versioned_object_uid().as_str();

        // NOTE: PostgreSQL docs, 9.28.10 "Advisory Lock Functions": a
        // transaction-level lock serializes two runs over one composition.
        transaction
            .execute(
                "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
                &[&versioned_object_uid],
            )
            .await
            .map_err(database(Step::Lock))?;

        let earlier = earlier_rows(&transaction, bridge, versioned_object_uid).await?;
        delete_earlier(&transaction, cdm, bridge, versioned_object_uid, &earlier).await?;

        let ids = assign_ids(&transaction, bridge, graph, &earlier).await?;
        let mut report = graph.report().clone();
        let mut resolver = Resolver {
            transaction: &transaction,
            bridge,
            create: *policy == PersonPolicy::CreateOnFirstSight,
            persons: BTreeMap::new(),
            ids: &ids,
        };
        let tables = stage_graph(&mut resolver, graph, &mut report).await?;
        record_keys(&transaction, bridge, graph, &ids).await?;
        for (table, rows) in tables.values() {
            copy_rows(&transaction, cdm, table, rows).await?;
        }
        let links = write_links(&transaction, cdm, bridge, graph, &ids).await?;
        report.record_links(links);

        transaction
            .execute(
                &format!(
                    "INSERT INTO {bridge}.watermark
                        (versioned_object_uid, ehr_id, version_uid, run_id, committed_at)
                     VALUES ($1, $2, $3, $4, now())
                     ON CONFLICT (versioned_object_uid) DO UPDATE SET
                        ehr_id = EXCLUDED.ehr_id, version_uid = EXCLUDED.version_uid,
                        run_id = EXCLUDED.run_id, committed_at = EXCLUDED.committed_at"
                ),
                &[
                    &versioned_object_uid,
                    &source.ehr_id().as_str(),
                    &source.version_uid().as_str(),
                    &run.as_str(),
                ],
            )
            .await
            .map_err(database(Step::Watermark))?;
        transaction.commit().await.map_err(database(Step::Commit))?;
        Ok(report)
    }

    /// Writes the derived visits and returns how many rows were written.
    ///
    /// Each visit keeps the id its key held before, or takes a new one from
    /// the `visit_occurrence` sequence; its earlier row is replaced. The
    /// person comes from the EHR under the writer's [`PersonPolicy`]. All
    /// visits commit in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::UnknownPerson`] under
    /// [`PersonPolicy::Existing`] for an EHR with no person,
    /// [`WriteError::SequenceExhausted`] when the sequence has no id left,
    /// and [`WriteError::Database`] for a statement PostgreSQL refuses; the
    /// transaction is rolled back in every case.
    pub async fn write_visits(
        &mut self,
        visits: &[Visit],
        concepts: VisitConcepts,
    ) -> Result<u64, WriteError> {
        let Self {
            client,
            cdm,
            bridge,
            policy,
            ..
        } = self;
        let table = graph::cdm_table("visit_occurrence")?;
        let transaction = client.transaction().await.map_err(database(Step::Begin))?;
        let create = *policy == PersonPolicy::CreateOnFirstSight;
        let mut ids = Vec::with_capacity(visits.len());
        let mut rows = Vec::with_capacity(visits.len());
        for visit in visits {
            let key = visit.key();
            let found = transaction
                .query_opt(
                    &format!(
                        "SELECT visit_occurrence_id FROM {bridge}.visit
                         WHERE ehr_id = $1 AND visit_source = $2"
                    ),
                    &[&key.ehr_id().as_str(), &key.source().as_str()],
                )
                .await
                .map_err(database(Step::Read))?;
            let id: i32 = if let Some(row) = found {
                row.try_get(0).map_err(database(Step::Read))?
            } else {
                let id = allocate(&transaction, bridge, table.name).await?;
                transaction
                    .execute(
                        &format!(
                            "INSERT INTO {bridge}.visit (ehr_id, visit_source, visit_occurrence_id)
                             VALUES ($1, $2, $3)"
                        ),
                        &[&key.ehr_id().as_str(), &key.source().as_str(), &id],
                    )
                    .await
                    .map_err(database(Step::Record))?;
                id
            };
            let person = person_id(&transaction, bridge, key.ehr_id(), create).await?;
            ids.push(id);
            rows.push(stage_row(
                table,
                BTreeMap::from([
                    ("visit_occurrence_id", Staged::Integer(Some(id))),
                    ("person_id", Staged::Integer(Some(person))),
                    (
                        "visit_concept_id",
                        Staged::Integer(Some(concepts.visit_concept_id)),
                    ),
                    (
                        "visit_start_date",
                        Staged::Text(Some(visit.start().as_str().to_owned())),
                    ),
                    (
                        "visit_start_datetime",
                        Staged::Text(visit.start_datetime().map(|at| at.as_str().to_owned())),
                    ),
                    (
                        "visit_end_date",
                        Staged::Text(Some(visit.end().as_str().to_owned())),
                    ),
                    (
                        "visit_end_datetime",
                        Staged::Text(visit.end_datetime().map(|at| at.as_str().to_owned())),
                    ),
                    (
                        "visit_type_concept_id",
                        Staged::Integer(Some(concepts.visit_type_concept_id)),
                    ),
                    (
                        "visit_source_value",
                        Staged::Text(Some(key.source().as_str().to_owned())),
                    ),
                ]),
            ));
        }
        transaction
            .execute(
                &format!("DELETE FROM {cdm}.visit_occurrence WHERE visit_occurrence_id = ANY($1)"),
                &[&ids],
            )
            .await
            .map_err(database(Step::Delete(table.name)))?;
        let written = copy_rows(&transaction, cdm, table, &rows).await?;
        transaction.commit().await.map_err(database(Step::Commit))?;
        Ok(written)
    }
}

/// The id of every row of one graph, by table and natural key.
type Ids<'g> = BTreeMap<(&'static str, &'g RecordKey), i32>;

/// The staged rows of one graph, by table.
type Tables = BTreeMap<&'static str, (&'static TableMeta, Vec<Vec<Staged>>)>;

/// Gives every row of `graph` the id its natural key held before, a person's
/// `person_id`, or a new id from its table's sequence.
async fn assign_ids<'g>(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    graph: &'g RecordGraph,
    earlier: &Earlier,
) -> Result<Ids<'g>, WriteError> {
    let mut ids = Ids::new();
    for row in graph.rows() {
        let table = row.table().name;
        let key = row.key();
        let id = if table == "person" {
            person_id(transaction, bridge, key.ehr_id(), true).await?
        } else if let Some(id) = earlier.ids.get(&(
            table,
            key.archetype_root_path().as_str().to_owned(),
            key.occurrence_path().as_str().to_owned(),
        )) {
            *id
        } else {
            allocate(transaction, bridge, table).await?
        };
        ids.insert((table, key), id);
    }
    Ok(ids)
}

/// Stages every row of `graph` with its references resolved, recording what
/// each mapping writes into `report`.
async fn stage_graph(
    resolver: &mut Resolver<'_, '_>,
    graph: &RecordGraph,
    report: &mut Report,
) -> Result<Tables, WriteError> {
    let mut tables = Tables::new();
    for row in graph.rows() {
        let table = row.table();
        let Some(key_column) = graph::primary_key(table) else {
            return Err(WriteError::SideTable {
                what: "a table with an integer key",
                value: table.name.to_owned(),
            });
        };
        let id = resolver
            .ids
            .get(&(table.name, row.key()))
            .copied()
            .ok_or_else(|| WriteError::UnknownRow {
                table: table.name,
                key: row.key().clone(),
            })?;
        let mut values = BTreeMap::from([(key_column.name, Staged::Integer(Some(id)))]);
        let mut concept_zero: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (column, cell) in row.cells() {
            let meta = table.column(column);
            let value = match cell {
                Cell::Value(value) => {
                    let concept =
                        meta.and_then(|meta| meta.foreign_key) == Some(("concept", "concept_id"));
                    if concept && matches!(value, Value::Integer(0)) {
                        concept_zero.insert(column, 1);
                    }
                    staged(value)
                }
                Cell::Reference(reference) => {
                    let target = meta
                        .and_then(|meta| meta.foreign_key)
                        .map_or(table.name, |(target, _)| target);
                    Staged::Integer(Some(resolver.resolve(reference, target).await?))
                }
            };
            values.insert(column, value);
        }
        report.record_written(row.mapping(), table.name, 1, &concept_zero);
        tables
            .entry(table.name)
            .or_insert_with(|| (table, Vec::new()))
            .1
            .push(stage_row(table, values));
    }
    Ok(tables)
}

/// The rows an earlier version of a composition wrote, from the side table.
struct Earlier {
    /// The id of each natural key, by table, archetype root and occurrence.
    ids: BTreeMap<(&'static str, String, String), i32>,
}

/// Reads the side table rows of `versioned_object_uid`.
async fn earlier_rows(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    versioned_object_uid: &str,
) -> Result<Earlier, WriteError> {
    let rows = transaction
        .query(
            &format!(
                "SELECT cdm_table, archetype_root_path, occurrence_path, surrogate_id
                 FROM {bridge}.record WHERE versioned_object_uid = $1"
            ),
            &[&versioned_object_uid],
        )
        .await
        .map_err(database(Step::Read))?;
    let mut ids = BTreeMap::new();
    for row in rows {
        let table: String = row.try_get(0).map_err(database(Step::Read))?;
        let root: String = row.try_get(1).map_err(database(Step::Read))?;
        let occurrence: String = row.try_get(2).map_err(database(Step::Read))?;
        let id: i32 = row.try_get(3).map_err(database(Step::Read))?;
        let meta = graph::cdm_table(&table)
            .ok()
            .filter(|meta| graph::primary_key(meta).is_some())
            .ok_or_else(|| WriteError::SideTable {
                what: "a CDM table with an integer key",
                value: table.clone(),
            })?;
        ids.insert((meta.name, root, occurrence), id);
    }
    Ok(Earlier { ids })
}

/// Deletes the CDM rows, the links and the side table rows an earlier
/// version of the composition wrote.
async fn delete_earlier(
    transaction: &Transaction<'_>,
    cdm: &SchemaName,
    bridge: &SchemaName,
    versioned_object_uid: &str,
    earlier: &Earlier,
) -> Result<(), WriteError> {
    let mut by_table: BTreeMap<&'static str, Vec<i32>> = BTreeMap::new();
    for ((table, _, _), id) in &earlier.ids {
        by_table.entry(table).or_default().push(*id);
    }
    for (table, ids) in &by_table {
        let meta = graph::cdm_table(table)?;
        let Some(key) = graph::primary_key(meta) else {
            return Err(WriteError::SideTable {
                what: "a table with an integer key",
                value: (*table).to_owned(),
            });
        };
        transaction
            .execute(
                &format!(
                    "DELETE FROM {cdm}.{} WHERE {} = ANY($1)",
                    quoted(table),
                    quoted(key.name)
                ),
                &[ids],
            )
            .await
            .map_err(database(Step::Delete(table)))?;
    }
    transaction
        .execute(
            &format!(
                "DELETE FROM {cdm}.fact_relationship AS f USING {bridge}.link AS l
                 WHERE l.versioned_object_uid = $1
                   AND f.domain_concept_id_1 = l.domain_concept_id_1
                   AND f.fact_id_1 = l.fact_id_1
                   AND f.domain_concept_id_2 = l.domain_concept_id_2
                   AND f.fact_id_2 = l.fact_id_2
                   AND f.relationship_concept_id = l.relationship_concept_id"
            ),
            &[&versioned_object_uid],
        )
        .await
        .map_err(database(Step::Delete("fact_relationship")))?;
    for table in ["link", "record"] {
        transaction
            .execute(
                &format!("DELETE FROM {bridge}.{table} WHERE versioned_object_uid = $1"),
                &[&versioned_object_uid],
            )
            .await
            .map_err(database(Step::Record))?;
    }
    Ok(())
}

/// Records the natural key and id of every row of `graph`.
async fn record_keys(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    graph: &RecordGraph,
    ids: &BTreeMap<(&'static str, &RecordKey), i32>,
) -> Result<(), WriteError> {
    let mut columns: [Vec<&str>; 6] = Default::default();
    let mut surrogate = Vec::new();
    for row in graph.rows() {
        let key = row.key();
        let [vo, root, occurrence, table, ehr, mapping] = &mut columns;
        vo.push(key.versioned_object_uid().as_str());
        root.push(key.archetype_root_path().as_str());
        occurrence.push(key.occurrence_path().as_str());
        table.push(row.table().name);
        ehr.push(key.ehr_id().as_str());
        mapping.push(row.mapping().as_str());
        let id =
            ids.get(&(row.table().name, key))
                .copied()
                .ok_or_else(|| WriteError::UnknownRow {
                    table: row.table().name,
                    key: key.clone(),
                })?;
        surrogate.push(id);
    }
    let [vo, root, occurrence, table, ehr, mapping] = &columns;
    transaction
        .execute(
            &format!(
                "INSERT INTO {bridge}.record (versioned_object_uid, archetype_root_path,
                    occurrence_path, cdm_table, ehr_id, mapping, surrogate_id)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[],
                    $5::text[], $6::text[], $7::integer[])"
            ),
            &[vo, root, occurrence, table, ehr, mapping, &surrogate],
        )
        .await
        .map_err(database(Step::Record))?;
    Ok(())
}

/// Writes both directions of every link of `graph` and returns the number of
/// `FACT_RELATIONSHIP` rows written.
async fn write_links(
    transaction: &Transaction<'_>,
    cdm: &SchemaName,
    bridge: &SchemaName,
    graph: &RecordGraph,
    ids: &BTreeMap<(&'static str, &RecordKey), i32>,
) -> Result<u64, WriteError> {
    let table = graph::cdm_table("fact_relationship")?;
    let id_of = |end: &graph::LinkEnd| {
        ids.get(&(end.table().name, end.key()))
            .copied()
            .ok_or_else(|| WriteError::UnknownRow {
                table: end.table().name,
                key: end.key().clone(),
            })
    };
    let mut rows = Vec::new();
    let mut side: [Vec<i32>; 5] = Default::default();
    for link in graph.links() {
        let first = (link.first().domain_concept_id(), id_of(link.first())?);
        let second = (link.second().domain_concept_id(), id_of(link.second())?);
        for (from, to) in [(first, second), (second, first)] {
            let fields = [
                from.0,
                from.1,
                to.0,
                to.1,
                graph::Link::RELATIONSHIP_CONCEPT_ID,
            ];
            for (column, value) in side.iter_mut().zip(fields) {
                column.push(value);
            }
            rows.push(stage_row(
                table,
                BTreeMap::from([
                    ("domain_concept_id_1", Staged::Integer(Some(from.0))),
                    ("fact_id_1", Staged::Integer(Some(from.1))),
                    ("domain_concept_id_2", Staged::Integer(Some(to.0))),
                    ("fact_id_2", Staged::Integer(Some(to.1))),
                    (
                        "relationship_concept_id",
                        Staged::Integer(Some(graph::Link::RELATIONSHIP_CONCEPT_ID)),
                    ),
                ]),
            ));
        }
    }
    let written = copy_rows(transaction, cdm, table, &rows).await?;
    let [d1, f1, d2, f2, relationship] = &side;
    transaction
        .execute(
            &format!(
                "INSERT INTO {bridge}.link (versioned_object_uid, domain_concept_id_1, fact_id_1,
                    domain_concept_id_2, fact_id_2, relationship_concept_id)
                 SELECT $1, * FROM UNNEST($2::integer[], $3::integer[], $4::integer[],
                    $5::integer[], $6::integer[])"
            ),
            &[
                &graph.source().versioned_object_uid().as_str(),
                d1,
                f1,
                d2,
                f2,
                relationship,
            ],
        )
        .await
        .map_err(database(Step::Record))?;
    Ok(written)
}

/// Resolves the references of one commit.
struct Resolver<'a, 't> {
    transaction: &'a Transaction<'t>,
    bridge: &'a SchemaName,
    create: bool,
    persons: BTreeMap<EhrId, i32>,
    ids: &'a BTreeMap<(&'static str, &'a RecordKey), i32>,
}

impl Resolver<'_, '_> {
    /// Returns the id `reference` names in the table `target`.
    async fn resolve(&mut self, reference: &Reference, target: &str) -> Result<i32, WriteError> {
        match reference {
            Reference::Person(ehr_id) => {
                if let Some(id) = self.persons.get(ehr_id) {
                    return Ok(*id);
                }
                let id = person_id(self.transaction, self.bridge, ehr_id, self.create).await?;
                self.persons.insert(ehr_id.clone(), id);
                Ok(id)
            }
            Reference::Visit(key) => {
                let found = self
                    .transaction
                    .query_opt(
                        &format!(
                            "SELECT visit_occurrence_id FROM {}.visit
                             WHERE ehr_id = $1 AND visit_source = $2",
                            self.bridge
                        ),
                        &[&key.ehr_id().as_str(), &key.source().as_str()],
                    )
                    .await
                    .map_err(database(Step::Read))?;
                match found {
                    Some(row) => row.try_get(0).map_err(database(Step::Read)),
                    None => Err(WriteError::UnknownVisit { key: key.clone() }),
                }
            }
            Reference::Row(key) => {
                let table = graph::cdm_table(target)?;
                if let Some(id) = self.ids.get(&(table.name, key)) {
                    return Ok(*id);
                }
                let found = self
                    .transaction
                    .query_opt(
                        &format!(
                            "SELECT surrogate_id FROM {}.record
                             WHERE versioned_object_uid = $1 AND archetype_root_path = $2
                               AND occurrence_path = $3 AND cdm_table = $4",
                            self.bridge
                        ),
                        &[
                            &key.versioned_object_uid().as_str(),
                            &key.archetype_root_path().as_str(),
                            &key.occurrence_path().as_str(),
                            &table.name,
                        ],
                    )
                    .await
                    .map_err(database(Step::Read))?;
                match found {
                    Some(row) => row.try_get(0).map_err(database(Step::Read)),
                    None => Err(WriteError::UnknownRow {
                        table: table.name,
                        key: key.clone(),
                    }),
                }
            }
        }
    }
}
