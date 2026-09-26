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

pub mod error;
pub mod input;
pub mod side_table;
pub mod stage;

use std::collections::BTreeMap;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use tokio_postgres::Client;
use tokio_postgres::Transaction;
use tokio_postgres::error::SqlState;

use crate::connection::CdmConnection;
use crate::ddl::SchemaName;
use crate::graph;
use crate::graph::RecordGraph;
use crate::graph::key::RecordKey;
use crate::graph::key::Visit;
use crate::graph::report::Report;
use crate::graph::row::Reference;

use crate::writer::error::Step;
use crate::writer::error::WriteError;
use crate::writer::input::PersonPolicy;
use crate::writer::input::RunId;
use crate::writer::input::VisitConcepts;
use crate::writer::side_table::allocate;
use crate::writer::side_table::assign_ids;
use crate::writer::side_table::bridge_ddl;
use crate::writer::side_table::delete_earlier;
use crate::writer::side_table::earlier_rows;
use crate::writer::side_table::person_id;
use crate::writer::side_table::record_keys;
use crate::writer::side_table::write_links;
use crate::writer::stage::Staged;
use crate::writer::stage::copy_rows;
use crate::writer::stage::stage_graph;
use crate::writer::stage::stage_row;

/// The version of a composition last committed, and the run that did it.
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    version_uid: ObjectVersionId,
    run_id: RunId,
}

impl Watermark {
    /// Returns the version committed.
    #[must_use]
    pub fn version_uid(&self) -> &ObjectVersionId {
        &self.version_uid
    }

    /// Returns the run that committed it.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
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
    /// The URL's `sslmode` is honoured as [`CdmConnection::new`] reads it,
    /// with no CA configured; [`CdmWriter::connect_with`] takes one.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::Url`] when the URL is refused, and
    /// [`WriteError::Connect`] when the database cannot be reached, refuses
    /// the login, or fails the TLS handshake.
    pub async fn connect(
        url: &str,
        cdm: SchemaName,
        bridge: SchemaName,
        policy: PersonPolicy,
    ) -> Result<Self, WriteError> {
        Self::connect_with(&CdmConnection::new(url, None)?, cdm, bridge, policy).await
    }

    /// Connects to the database `connection` names, over the TLS it settles.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::Connect`] when the database cannot be reached,
    /// refuses the login, or fails the TLS handshake.
    pub async fn connect_with(
        connection: &CdmConnection,
        cdm: SchemaName,
        bridge: SchemaName,
        policy: PersonPolicy,
    ) -> Result<Self, WriteError> {
        let (config, tls) = connection.writer();
        let (client, connection) = config
            .connect(tls)
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
    /// [`WriteError::Identifier`] when it holds an empty run id, and
    /// [`WriteError::Version`] when its version is no `OBJECT_VERSION_ID`.
    pub async fn watermark(
        &self,
        versioned_object_uid: &HierObjectId,
    ) -> Result<Option<Watermark>, WriteError> {
        let row = self
            .client
            .query_opt(
                &format!(
                    "SELECT version_uid, run_id FROM {}.watermark WHERE versioned_object_uid = $1",
                    self.bridge
                ),
                &[&versioned_object_uid.value()],
            )
            .await
            .map_err(database(Step::Read))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let version: String = row.try_get(0).map_err(database(Step::Read))?;
        let run: String = row.try_get(1).map_err(database(Step::Read))?;
        Ok(Some(Watermark {
            version_uid: ObjectVersionId::new(version.as_str()).map_err(|source| {
                WriteError::Version {
                    value: version.clone(),
                    source,
                }
            })?,
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
        let versioned_object_uid = source.versioned_object_uid().value();

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
                    &source.ehr_id().value(),
                    &source.version_uid().value(),
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
        let table = graph::row::cdm_table("visit_occurrence")?;
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
                    &[&key.ehr_id().value(), &key.source().as_str()],
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
                        &[&key.ehr_id().value(), &key.source().as_str(), &id],
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

/// Resolves the references of one commit.
struct Resolver<'a, 't> {
    transaction: &'a Transaction<'t>,
    bridge: &'a SchemaName,
    create: bool,
    persons: BTreeMap<String, i32>,
    ids: &'a BTreeMap<(&'static str, &'a RecordKey), i32>,
}

impl Resolver<'_, '_> {
    /// Returns the id `reference` names in the table `target`.
    async fn resolve(&mut self, reference: &Reference, target: &str) -> Result<i32, WriteError> {
        match reference {
            Reference::Person(ehr_id) => {
                if let Some(id) = self.persons.get(ehr_id.value()) {
                    return Ok(*id);
                }
                let id = person_id(self.transaction, self.bridge, ehr_id, self.create).await?;
                self.persons.insert(ehr_id.value().to_owned(), id);
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
                        &[&key.ehr_id().value(), &key.source().as_str()],
                    )
                    .await
                    .map_err(database(Step::Read))?;
                match found {
                    Some(row) => row.try_get(0).map_err(database(Step::Read)),
                    None => Err(WriteError::UnknownVisit { key: key.clone() }),
                }
            }
            Reference::Row(key) => {
                let table = graph::row::cdm_table(target)?;
                if let Some(id) = self.ids.get(&(table.name, key)) {
                    return Ok(*id);
                }
                let found = self
                    .transaction
                    .query_opt(
                        &format!(
                            "SELECT surrogate_id FROM {}.record
                             WHERE versioned_object_uid = $1 AND archetype_root_path = $2
                               AND occurrence_path = $3 AND cdm_table = $4
                               AND mapping = $5 AND entry = $6 AND branch = $7",
                            self.bridge
                        ),
                        &[
                            &key.versioned_object_uid().value(),
                            &key.archetype_root_path().as_str(),
                            &key.occurrence_path().as_str(),
                            &table.name,
                            &key.discriminator().mapping().as_str(),
                            &i32::from(key.discriminator().entry()),
                            &i32::from(key.discriminator().branch()),
                        ],
                    )
                    .await
                    .map_err(database(Step::Read))?;
                match found {
                    Some(row) => row.try_get(0).map_err(database(Step::Read)),
                    None => Err(WriteError::UnknownRow {
                        table: table.name,
                        key: Box::new(key.clone()),
                    }),
                }
            }
        }
    }
}
