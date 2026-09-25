// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The derived tables against a PostgreSQL CDM schema the embedded DDL
//! builds: the observation period from the first and last clinical event
//! (FerroBRIDGE's own), and the two era scripts the CDM publishes
//! (<https://ohdsi.github.io/CommonDataModel/sqlScripts.html>) in their
//! PostgreSQL form. The container-backed tests run only when
//! `FERROBRIDGE_E2E=1` admits the harness.

use ferrobridge_testkit::containers::{self, Postgres};
use omop_cdm::database::{self, CdmPool};
use omop_cdm::ddl::SchemaName;
use omop_cdm::derived;
use omop_cdm::graph::{
    ArchetypeRootPath, EhrId, MappingName, OccurrencePath, RecordGraph, RecordKey, Reference, Row,
    Source, Value, VersionUid, VersionedObjectUid,
};
use omop_cdm::value::CdmDate;
use omop_cdm::writer::{CdmWriter, PersonPolicy, RunId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::error::Error;

/// The synthetic EHR.
const EHR: &str = "2b0c4e8e-44a4-4a38-9d86-3c7d0f3a51e0";

/// The synthetic versioned composition.
const COMPOSITION: &str = "c9a3f0f2-54d3-4c43-9a33-0d5b4f4f2d11";

/// The synthetic RxNorm-class ingredient the drug era script rolls up to.
const INGREDIENT: i32 = 5001;

/// Starts PostgreSQL and returns it with a writer over a fresh CDM.
async fn database() -> Result<(Postgres, CdmWriter), Box<dyn Error>> {
    let postgres = containers::postgres().await?;
    let options: PgConnectOptions = postgres.url().parse()?;
    let pool = CdmPool::connect(
        PgPoolOptions::new().max_connections(1),
        options,
        SchemaName::new("cdm")?,
    )
    .await?;
    database::init(&pool).await?;
    pool.pool().close().await;
    let mut writer = CdmWriter::connect(
        postgres.url(),
        SchemaName::new("cdm")?,
        SchemaName::new("ferrobridge")?,
        PersonPolicy::CreateOnFirstSight,
    )
    .await?;
    writer.init().await?;
    Ok((postgres, writer))
}

/// Returns the key of the node at `occurrence`.
fn key(occurrence: &str) -> Result<RecordKey, Box<dyn Error>> {
    Ok(RecordKey::new(
        EhrId::new(EHR)?,
        VersionedObjectUid::new(COMPOSITION)?,
        ArchetypeRootPath::new("/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]")?,
        OccurrencePath::new(occurrence)?,
    ))
}

/// Returns a date value.
fn date(text: &str) -> Result<Value, Box<dyn Error>> {
    Ok(Value::Date(CdmDate::new(text)?))
}

/// Returns a graph with two conditions and two drug exposures 20 days
/// apart, and a measurement on its own date.
fn clinical() -> Result<RecordGraph, Box<dyn Error>> {
    let mapping = MappingName::new("synthetic")?;
    let person =
        || -> Result<Reference, Box<dyn Error>> { Ok(Reference::Person(EhrId::new(EHR)?)) };
    let mut graph = RecordGraph::new(Source::new(
        EhrId::new(EHR)?,
        VersionedObjectUid::new(COMPOSITION)?,
        VersionUid::new(format!("{COMPOSITION}::ferrobridge.test::1"))?,
    ));
    for (index, (start, end)) in [("2026-05-01", "2026-05-10"), ("2026-05-30", "2026-06-02")]
        .into_iter()
        .enumerate()
    {
        graph.push_row(
            Row::builder(
                "condition_occurrence",
                key(&format!("/c{index}"))?,
                mapping.clone(),
            )?
            .reference("person_id", person()?)?
            .value("condition_concept_id", Value::Integer(1002))?
            .value("condition_start_date", date(start)?)?
            .value("condition_end_date", date(end)?)?
            .value("condition_type_concept_id", Value::Integer(32817))?
            .build()?,
        )?;
        graph.push_row(
            Row::builder(
                "drug_exposure",
                key(&format!("/d{index}"))?,
                mapping.clone(),
            )?
            .reference("person_id", person()?)?
            .value("drug_concept_id", Value::Integer(1010))?
            .value("drug_exposure_start_date", date(start)?)?
            .value("drug_exposure_end_date", date(end)?)?
            .value("drug_type_concept_id", Value::Integer(32817))?
            .build()?,
        )?;
    }
    graph.push_row(
        Row::builder("measurement", key("/m")?, mapping)?
            .reference("person_id", person()?)?
            .value("measurement_concept_id", Value::Integer(1001))?
            .value("measurement_date", date("2026-06-20")?)?
            .value("measurement_type_concept_id", Value::Integer(32817))?
            .build()?,
    )?;
    Ok(graph)
}

/// Reads one row of `query` as text columns.
async fn row(postgres: &Postgres, query: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let rows = client.query(query, &[]).await?;
    let [row] = rows.as_slice() else {
        return Err(format!("{query} answered {} rows, not one", rows.len()).into());
    };
    Ok((0..row.len())
        .map(|index| row.get::<_, String>(index))
        .collect())
}

#[tokio::test]
async fn the_observation_period_spans_the_first_to_the_last_event() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let (postgres, mut writer) = database().await?;
    writer.commit(&clinical()?, &RunId::new("run-1")?).await?;
    assert_eq!(1, derived::observation_period(&mut writer, 32880).await?);
    assert_eq!(
        1,
        derived::observation_period(&mut writer, 32880).await?,
        "a rebuild replaces"
    );
    assert_eq!(
        vec!["1", "1", "2026-05-01", "2026-06-20", "32880"],
        row(
            &postgres,
            "SELECT observation_period_id::text, person_id::text,
                    observation_period_start_date::text, observation_period_end_date::text,
                    period_type_concept_id::text
             FROM cdm.observation_period"
        )
        .await?
    );
    Ok(())
}

#[tokio::test]
async fn the_condition_era_script_joins_conditions_within_thirty_days() -> Result<(), Box<dyn Error>>
{
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let (postgres, mut writer) = database().await?;
    writer.commit(&clinical()?, &RunId::new("run-1")?).await?;
    assert_eq!(1, derived::condition_era(&mut writer).await?);
    assert_eq!(
        1,
        derived::condition_era(&mut writer).await?,
        "a rebuild replaces"
    );
    assert_eq!(
        vec!["1", "1002", "2026-05-01", "2026-06-02", "2"],
        row(
            &postgres,
            "SELECT person_id::text, condition_concept_id::text, condition_era_start_date::text,
                    condition_era_end_date::text, condition_occurrence_count::text
             FROM cdm.condition_era"
        )
        .await?
    );
    Ok(())
}

#[tokio::test]
async fn the_drug_era_script_rolls_exposures_up_to_their_ingredient() -> Result<(), Box<dyn Error>>
{
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let (postgres, mut writer) = database().await?;
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    // The published script reads ingredients of vocabulary_id 'RxNorm' only, so
    // this test writes one synthetic concept under that id; no RxNorm content.
    client
        .batch_execute(&format!(
            "INSERT INTO cdm.concept VALUES ({INGREDIENT}, 'Synthetic ingredient', 'Drug',
                 'RxNorm', 'Ingredient', 'S', 'FB-SYN-1', '1970-01-01', '2099-12-31', NULL);
             INSERT INTO cdm.concept_ancestor VALUES ({INGREDIENT}, 1010, 1, 1)"
        ))
        .await?;
    writer.commit(&clinical()?, &RunId::new("run-1")?).await?;
    assert_eq!(1, derived::drug_era(&mut writer).await?);
    assert_eq!(
        vec!["1", "5001", "2026-05-01", "2026-06-02", "2", "20"],
        row(
            &postgres,
            "SELECT person_id::text, drug_concept_id::text, drug_era_start_date::text,
                    drug_era_end_date::text, drug_exposure_count::text, gap_days::text
             FROM cdm.drug_era"
        )
        .await?
    );
    Ok(())
}
