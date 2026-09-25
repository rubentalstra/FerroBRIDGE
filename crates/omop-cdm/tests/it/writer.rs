// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDM writer against a PostgreSQL CDM schema the embedded DDL builds.
//!
//! The rules are FerroBRIDGE's own (no specification governs keys or
//! re-runs): one composition's graph commits whole or not at all, a natural
//! key keeps its id across commits, a later version replaces the earlier
//! version's rows, and a link is written in both directions with
//! `relationship_concept_id` 0. The container-backed tests run only when
//! `FERROBRIDGE_E2E=1` admits the harness.

use ferrobridge_testkit::containers::{self, Postgres};
use omop_cdm::database::{self, CdmPool};
use omop_cdm::ddl::SchemaName;
use omop_cdm::graph::{
    ArchetypeRootPath, Discriminator, EhrId, Link, LinkEnd, MappingName, OccurrencePath,
    RecordGraph, RecordKey, Reference, Row, Source, Value, VersionUid, VersionedObjectUid, Visit,
    VisitKey, VisitSource,
};
use omop_cdm::value::CdmDate;
use omop_cdm::writer::{CdmWriter, PersonPolicy, RunId, VisitConcepts, WriteError};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::error::Error;

/// The CDM schema the tests write.
const CDM: &str = "cdm";

/// The bridge schema the tests keep the side table in.
const BRIDGE: &str = "ferrobridge";

/// The synthetic EHR.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// The synthetic versioned composition.
const COMPOSITION: &str = "8849182c-82ad-4088-a07f-48ead4180515";

/// The archetype root of the synthetic laboratory result.
const ROOT: &str = "/content[openEHR-EHR-OBSERVATION.laboratory_test_result.v1]";

/// The `domain_concept_id` the synthetic links carry for a measurement.
const MEASUREMENT_DOMAIN: i32 = 9011;

/// A started database with the CDM schema and the bridge schema in place.
struct Database {
    /// Kept alive for the test; dropping it stops the container.
    postgres: Postgres,
    writer: CdmWriter,
}

/// Starts PostgreSQL, builds the CDM schema and the bridge schema.
async fn database(policy: PersonPolicy) -> Result<Database, Box<dyn Error>> {
    let postgres = containers::postgres().await?;
    let options: PgConnectOptions = postgres.url().parse()?;
    let pool = CdmPool::connect(
        PgPoolOptions::new().max_connections(1),
        options,
        SchemaName::new(CDM)?,
    )
    .await?;
    database::init(&pool).await?;
    pool.pool().close().await;
    let mut writer = CdmWriter::connect(
        postgres.url(),
        SchemaName::new(CDM)?,
        SchemaName::new(BRIDGE)?,
        policy,
    )
    .await?;
    writer.init().await?;
    Ok(Database { postgres, writer })
}

/// Opens a second connection for reading back what the writer wrote.
async fn reader(postgres: &Postgres) -> Result<tokio_postgres::Client, Box<dyn Error>> {
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    Ok(client)
}

/// Returns the key of an analyte at `index` of the synthetic result.
fn analyte(index: u32) -> Result<RecordKey, Box<dyn Error>> {
    Ok(RecordKey::new(
        EhrId::new(EHR)?,
        VersionedObjectUid::new(COMPOSITION)?,
        ArchetypeRootPath::new(ROOT)?,
        OccurrencePath::new(format!(
            "/data[at0001]/events[at0002]/data[at0003]/items[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1,{index}]"
        ))?,
        Discriminator::new(MappingName::new("Laboratory_test_result_v1")?, 0, 0),
    ))
}

/// Returns the key of the synthetic result itself.
fn result() -> Result<RecordKey, Box<dyn Error>> {
    Ok(RecordKey::new(
        EhrId::new(EHR)?,
        VersionedObjectUid::new(COMPOSITION)?,
        ArchetypeRootPath::new(ROOT)?,
        OccurrencePath::new("/")?,
        Discriminator::new(MappingName::new("Laboratory_test_result_v1")?, 0, 0),
    ))
}

/// Returns a measurement under `key` with `concept` and `value`.
fn measurement(key: RecordKey, concept: i32, value: Option<f64>) -> Result<Row, Box<dyn Error>> {
    Ok(Row::builder("measurement", key)?
        .reference("person_id", Reference::Person(EhrId::new(EHR)?))?
        .value("measurement_concept_id", Value::Integer(concept))?
        .value("measurement_date", Value::Date(CdmDate::new("2026-06-15")?))?
        .value("measurement_type_concept_id", Value::Integer(32817))?
        .optional("value_as_number", value.map(Value::Float))?
        .value("unit_concept_id", Value::Integer(3001))?
        .build()?)
}

/// Returns the graph of version `version` with the result and `analytes`
/// analytes, each linked to the result.
fn laboratory(version: u32, analytes: u32) -> Result<RecordGraph, Box<dyn Error>> {
    let mut graph = RecordGraph::new(Source::new(
        EhrId::new(EHR)?,
        VersionedObjectUid::new(COMPOSITION)?,
        VersionUid::new(format!("{COMPOSITION}::ferrobridge.test::{version}"))?,
    ));
    graph.push_row(measurement(result()?, 1001, None)?)?;
    for index in 1..=analytes {
        let concept = if index == 1 { 1001 } else { 0 };
        graph.push_row(measurement(analyte(index)?, concept, Some(4.5))?)?;
        graph.push_link(Link::new(
            LinkEnd::new("measurement", result()?, MEASUREMENT_DOMAIN)?,
            LinkEnd::new("measurement", analyte(index)?, MEASUREMENT_DOMAIN)?,
        ))?;
    }
    Ok(graph)
}

/// Returns every measurement as `(id, concept, value)`, ordered by id.
async fn measurements(
    client: &tokio_postgres::Client,
) -> Result<Vec<(i32, i32, Option<String>)>, Box<dyn Error>> {
    let rows = client
        .query(
            "SELECT measurement_id, measurement_concept_id, value_as_number::text
             FROM cdm.measurement ORDER BY measurement_id",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

/// Returns the number of rows of `table`.
async fn count(client: &tokio_postgres::Client, table: &str) -> Result<i64, Box<dyn Error>> {
    Ok(client
        .query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await?
        .get(0))
}

#[tokio::test]
async fn a_graph_commits_its_rows_links_and_watermark() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::CreateOnFirstSight).await?;
    let run = RunId::new("run-1")?;
    let report = database.writer.commit(&laboratory(1, 2)?, &run).await?;
    let client = reader(&database.postgres).await?;

    assert_eq!(
        vec![
            (1, 1001, None),
            (2, 1001, Some(String::from("4.5"))),
            (3, 0, Some(String::from("4.5"))),
        ],
        measurements(&client).await?
    );
    assert_eq!(
        4,
        count(&client, "cdm.fact_relationship").await?,
        "two links, both directions"
    );
    assert_eq!(
        0,
        count(
            &client,
            "cdm.fact_relationship WHERE relationship_concept_id <> 0"
        )
        .await?
    );
    assert_eq!(1, count(&client, "ferrobridge.person_map").await?);
    assert_eq!(
        Some(&3),
        report.rows().get(&(
            MappingName::new("Laboratory_test_result_v1")?,
            "measurement"
        ))
    );
    assert_eq!(
        Some(&1),
        report.concept_zero().get(&(
            MappingName::new("Laboratory_test_result_v1")?,
            "measurement",
            "measurement_concept_id"
        ))
    );
    assert_eq!(4, report.zero_relationship_links());
    let watermark = database
        .writer
        .watermark(&VersionedObjectUid::new(COMPOSITION)?)
        .await?
        .ok_or("the commit advanced no watermark")?;
    assert_eq!(
        format!("{COMPOSITION}::ferrobridge.test::1"),
        watermark.version_uid().as_str()
    );
    assert_eq!("run-1", watermark.run_id().as_str());
    Ok(())
}

#[tokio::test]
async fn committing_the_same_graph_again_keeps_every_id() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::CreateOnFirstSight).await?;
    let graph = laboratory(1, 2)?;
    database
        .writer
        .commit(&graph, &RunId::new("run-1")?)
        .await?;
    let client = reader(&database.postgres).await?;
    let first = measurements(&client).await?;
    database
        .writer
        .commit(&graph, &RunId::new("run-2")?)
        .await?;
    assert_eq!(first, measurements(&client).await?);
    assert_eq!(4, count(&client, "cdm.fact_relationship").await?);
    assert_eq!(3, count(&client, "ferrobridge.record").await?);
    Ok(())
}

#[tokio::test]
async fn a_later_version_replaces_the_rows_of_the_earlier_one() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::CreateOnFirstSight).await?;
    database
        .writer
        .commit(&laboratory(1, 2)?, &RunId::new("run-1")?)
        .await?;
    database
        .writer
        .commit(&laboratory(2, 1)?, &RunId::new("run-2")?)
        .await?;
    let client = reader(&database.postgres).await?;
    assert_eq!(
        vec![(1, 1001, None), (2, 1001, Some(String::from("4.5")))],
        measurements(&client).await?,
        "the second analyte of version 1 is gone and the rest keep their ids"
    );
    assert_eq!(2, count(&client, "cdm.fact_relationship").await?);
    Ok(())
}

#[tokio::test]
async fn an_unresolved_reference_writes_nothing_of_the_composition() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::CreateOnFirstSight).await?;
    let mut graph = laboratory(1, 1)?;
    let visit = VisitKey::new(EhrId::new(EHR)?, VisitSource::new("encounter-1")?);
    graph.push_row(
        Row::builder("measurement", analyte(9)?)?
            .reference("person_id", Reference::Person(EhrId::new(EHR)?))?
            .value("measurement_concept_id", Value::Integer(1001))?
            .value("measurement_date", Value::Date(CdmDate::new("2026-06-15")?))?
            .value("measurement_type_concept_id", Value::Integer(32817))?
            .reference("visit_occurrence_id", Reference::Visit(visit))?
            .build()?,
    )?;
    let error = database
        .writer
        .commit(&graph, &RunId::new("run-1")?)
        .await
        .expect_err("no visit was derived");
    assert!(
        matches!(error, WriteError::UnknownVisit { .. }),
        "{error:?}"
    );
    let client = reader(&database.postgres).await?;
    assert_eq!(0, count(&client, "cdm.measurement").await?);
    assert_eq!(
        0,
        count(&client, "ferrobridge.person_map").await?,
        "the person was rolled back too"
    );
    assert_eq!(0, count(&client, "ferrobridge.watermark").await?);
    Ok(())
}

#[tokio::test]
async fn an_exhausted_sequence_is_a_typed_error_and_rolls_back() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::CreateOnFirstSight).await?;
    let client = reader(&database.postgres).await?;
    client
        .batch_execute("ALTER SEQUENCE ferrobridge.measurement_id_seq RESTART WITH 2147483646")
        .await?;
    let error = database
        .writer
        .commit(&laboratory(1, 2)?, &RunId::new("run-1")?)
        .await
        .expect_err("three rows need three ids and two are left");
    assert!(
        matches!(
            error,
            WriteError::SequenceExhausted {
                table: "measurement",
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(0, count(&client, "cdm.measurement").await?);
    assert_eq!(0, count(&client, "ferrobridge.record").await?);
    Ok(())
}

#[tokio::test]
async fn the_existing_policy_refuses_an_ehr_without_a_person() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::Existing).await?;
    let error = database
        .writer
        .commit(&laboratory(1, 1)?, &RunId::new("run-1")?)
        .await
        .expect_err("no PERSON row names the EHR");
    assert!(
        matches!(error, WriteError::UnknownPerson { .. }),
        "{error:?}"
    );
    Ok(())
}

#[tokio::test]
async fn visits_keep_their_ids_and_resolve_references() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut database = database(PersonPolicy::CreateOnFirstSight).await?;
    let key = VisitKey::new(EhrId::new(EHR)?, VisitSource::new("encounter-1")?);
    let visit = Visit::new(
        key.clone(),
        (CdmDate::new("2026-06-14")?, None),
        (CdmDate::new("2026-06-16")?, None),
    )?;
    let concepts = VisitConcepts {
        visit_concept_id: 1001,
        visit_type_concept_id: 32817,
    };
    assert_eq!(
        1,
        database
            .writer
            .write_visits(std::slice::from_ref(&visit), concepts)
            .await?
    );
    assert_eq!(1, database.writer.write_visits(&[visit], concepts).await?);
    let client = reader(&database.postgres).await?;
    let row = client
        .query_one(
            "SELECT visit_occurrence_id, visit_source_value, visit_end_date::text
             FROM cdm.visit_occurrence",
            &[],
        )
        .await?;
    assert_eq!(1_i32, row.get::<_, i32>(0));
    assert_eq!("encounter-1", row.get::<_, &str>(1));
    assert_eq!("2026-06-16", row.get::<_, &str>(2));

    let mut graph = laboratory(1, 0)?;
    graph.push_row(
        Row::builder("measurement", analyte(1)?)?
            .reference("person_id", Reference::Person(EhrId::new(EHR)?))?
            .value("measurement_concept_id", Value::Integer(1001))?
            .value("measurement_date", Value::Date(CdmDate::new("2026-06-15")?))?
            .value("measurement_type_concept_id", Value::Integer(32817))?
            .reference("visit_occurrence_id", Reference::Visit(key))?
            .build()?,
    )?;
    database
        .writer
        .commit(&graph, &RunId::new("run-1")?)
        .await?;
    let linked: Option<i32> = client
        .query_one(
            "SELECT visit_occurrence_id FROM cdm.measurement WHERE measurement_id = 2",
            &[],
        )
        .await?
        .get(0);
    assert_eq!(Some(1), linked);
    Ok(())
}
