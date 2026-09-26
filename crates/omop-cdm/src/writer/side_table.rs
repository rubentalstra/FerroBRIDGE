// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The natural-key side table: the id sequences, the id of every natural key,
//! the rows an earlier version wrote, and the links of a composition.

use std::collections::BTreeMap;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use tokio_postgres::Transaction;

use crate::ddl::SchemaName;
use crate::graph;
use crate::graph::RecordGraph;
use crate::graph::key::RecordKey;
use crate::writer::database;
use crate::writer::error::Step;
use crate::writer::error::WriteError;
use crate::writer::quoted;
use crate::writer::stage::Staged;
use crate::writer::stage::copy_rows;
use crate::writer::stage::stage_row;

/// Returns the name of the sequence the ids of `table` come from.
pub(super) fn sequence(bridge: &SchemaName, table: &str) -> String {
    format!("{bridge}.{}", quoted(&format!("{table}_id_seq")))
}

/// Returns the next id of `table` from its sequence.
pub(super) async fn allocate(
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
pub(super) async fn person_id(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    ehr_id: &HierObjectId,
    create: bool,
) -> Result<i32, WriteError> {
    let found = transaction
        .query_opt(
            &format!("SELECT person_id FROM {bridge}.person_map WHERE ehr_id = $1"),
            &[&ehr_id.value()],
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
            &[&ehr_id.value(), &id],
        )
        .await
        .map_err(database(Step::Record))?;
    Ok(id)
}

/// The tables and sequences of the bridge schema.
pub(super) fn bridge_ddl(bridge: &SchemaName) -> String {
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
                entry integer NOT NULL,
                branch integer NOT NULL,
                surrogate_id integer NOT NULL,
                PRIMARY KEY (versioned_object_uid, archetype_root_path, occurrence_path,
                    mapping, entry, branch, cdm_table),
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
        if graph::row::cdm_table(table.name).is_ok() && graph::row::primary_key(table).is_some() {
            statements.push(format!(
                "CREATE SEQUENCE IF NOT EXISTS {} AS integer",
                sequence(bridge, table.name)
            ));
        }
    }
    statements.join(";\n")
}

/// The id of every row of one graph, by table and natural key.
pub(super) type Ids<'g> = BTreeMap<(&'static str, &'g RecordKey), i32>;

/// Gives every row of `graph` the id its natural key held before, a person's
/// `person_id`, or a new id from its table's sequence.
pub(super) async fn assign_ids<'g>(
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
        } else if let Some(id) = earlier.ids.get(&EarlierKey::of(table, key)) {
            *id
        } else {
            allocate(transaction, bridge, table).await?
        };
        ids.insert((table, key), id);
    }
    Ok(ids)
}

/// One natural key as the side table holds it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct EarlierKey {
    table: &'static str,
    root: String,
    occurrence: String,
    mapping: String,
    entry: i32,
    branch: i32,
}

impl EarlierKey {
    /// Returns the side table key of `key` in `table`.
    fn of(table: &'static str, key: &RecordKey) -> Self {
        let discriminator = key.discriminator();
        Self {
            table,
            root: key.archetype_root_path().as_str().to_owned(),
            occurrence: key.occurrence_path().as_str().to_owned(),
            mapping: discriminator.mapping().as_str().to_owned(),
            entry: i32::from(discriminator.entry()),
            branch: i32::from(discriminator.branch()),
        }
    }
}

/// The rows an earlier version of a composition wrote, from the side table.
pub(super) struct Earlier {
    /// The id of each natural key.
    ids: BTreeMap<EarlierKey, i32>,
}

/// Reads the side table rows of `versioned_object_uid`.
pub(super) async fn earlier_rows(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    versioned_object_uid: &str,
) -> Result<Earlier, WriteError> {
    let rows = transaction
        .query(
            &format!(
                "SELECT cdm_table, archetype_root_path, occurrence_path, mapping, entry, branch,
                    surrogate_id
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
        let mapping: String = row.try_get(3).map_err(database(Step::Read))?;
        let entry: i32 = row.try_get(4).map_err(database(Step::Read))?;
        let branch: i32 = row.try_get(5).map_err(database(Step::Read))?;
        let id: i32 = row.try_get(6).map_err(database(Step::Read))?;
        let meta = graph::row::cdm_table(&table)
            .ok()
            .filter(|meta| graph::row::primary_key(meta).is_some())
            .ok_or_else(|| WriteError::SideTable {
                what: "a CDM table with an integer key",
                value: table.clone(),
            })?;
        ids.insert(
            EarlierKey {
                table: meta.name,
                root,
                occurrence,
                mapping,
                entry,
                branch,
            },
            id,
        );
    }
    Ok(Earlier { ids })
}

/// Deletes the CDM rows, the links and the side table rows an earlier
/// version of the composition wrote.
pub(super) async fn delete_earlier(
    transaction: &Transaction<'_>,
    cdm: &SchemaName,
    bridge: &SchemaName,
    versioned_object_uid: &str,
    earlier: &Earlier,
) -> Result<(), WriteError> {
    let mut by_table: BTreeMap<&'static str, Vec<i32>> = BTreeMap::new();
    for (key, id) in &earlier.ids {
        by_table.entry(key.table).or_default().push(*id);
    }
    for (table, ids) in &by_table {
        let meta = graph::row::cdm_table(table)?;
        let Some(key) = graph::row::primary_key(meta) else {
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
pub(super) async fn record_keys(
    transaction: &Transaction<'_>,
    bridge: &SchemaName,
    graph: &RecordGraph,
    ids: &BTreeMap<(&'static str, &RecordKey), i32>,
) -> Result<(), WriteError> {
    let mut columns: [Vec<&str>; 6] = Default::default();
    let mut surrogate = Vec::new();
    let mut entries: Vec<i32> = Vec::new();
    let mut branches: Vec<i32> = Vec::new();
    for row in graph.rows() {
        let key = row.key();
        let [vo, root, occurrence, table, ehr, mapping] = &mut columns;
        vo.push(key.versioned_object_uid().value());
        root.push(key.archetype_root_path().as_str());
        occurrence.push(key.occurrence_path().as_str());
        table.push(row.table().name);
        ehr.push(key.ehr_id().value());
        mapping.push(key.discriminator().mapping().as_str());
        entries.push(i32::from(key.discriminator().entry()));
        branches.push(i32::from(key.discriminator().branch()));
        let id =
            ids.get(&(row.table().name, key))
                .copied()
                .ok_or_else(|| WriteError::UnknownRow {
                    table: row.table().name,
                    key: Box::new(key.clone()),
                })?;
        surrogate.push(id);
    }
    let [vo, root, occurrence, table, ehr, mapping] = &columns;
    transaction
        .execute(
            &format!(
                "INSERT INTO {bridge}.record (versioned_object_uid, archetype_root_path,
                    occurrence_path, cdm_table, ehr_id, mapping, entry, branch, surrogate_id)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[],
                    $5::text[], $6::text[], $7::integer[], $8::integer[], $9::integer[])"
            ),
            &[
                vo, root, occurrence, table, ehr, mapping, &entries, &branches, &surrogate,
            ],
        )
        .await
        .map_err(database(Step::Record))?;
    Ok(())
}

/// Writes both directions of every link of `graph` and returns the number of
/// `FACT_RELATIONSHIP` rows written.
pub(super) async fn write_links(
    transaction: &Transaction<'_>,
    cdm: &SchemaName,
    bridge: &SchemaName,
    graph: &RecordGraph,
    ids: &BTreeMap<(&'static str, &RecordKey), i32>,
) -> Result<u64, WriteError> {
    let table = graph::row::cdm_table("fact_relationship")?;
    let id_of = |end: &graph::link::LinkEnd| {
        ids.get(&(end.table().name, end.key()))
            .copied()
            .ok_or_else(|| WriteError::UnknownRow {
                table: end.table().name,
                key: Box::new(end.key().clone()),
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
                graph::link::Link::RELATIONSHIP_CONCEPT_ID,
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
                        Staged::Integer(Some(graph::link::Link::RELATIONSHIP_CONCEPT_ID)),
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
                &graph.source().versioned_object_uid().value(),
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
