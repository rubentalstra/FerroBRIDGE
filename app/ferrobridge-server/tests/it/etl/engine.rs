// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The run over the OMOCL engine and the laboratory corpus files.

use ferrobridge_server::etl::report::RunReport;
use ferrobridge_server::etl::{RunOptions, run};
use ferrobridge_testkit::containers::{self, Postgres};
use omop_cdm::database::{self, CdmPool};
use omop_cdm::ddl::SchemaName;
use omop_cdm::writer::{CdmWriter, input::PersonPolicy, input::RunId};
use serde_json::json;
use std::error::Error;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::COMPOSITIONS_UIDS;
use super::EHR;
use super::client;
use super::count;
use super::dump;
use super::settings;

/// The OMOCL corpus files the laboratory run maps with.
const LABORATORY_FILES: [&str; 3] = [
    "medical_data/observation/Laboratory_test_result_v1.yml",
    "medical_data/cluster/Laboratory_test_analyte_v1.yml",
    "medical_data/cluster/Specimen_v1.yml",
];

/// The mapper the laboratory run maps with.
type EngineMapper =
    ferrobridge_server::etl::mapper::OmoclMapper<ferrobridge_server::etl::mapper::CdmVocabulary>;

/// A CDM database with the synthetic vocabulary loaded, a stubbed CDR serving
/// two laboratory compositions, and the OMOCL mapper over the corpus files.
struct LaboratoryHarness {
    /// Kept alive for the case; dropping it stops the container.
    postgres: Postgres,
    cdr: MockServer,
    writer: CdmWriter,
    mapper: EngineMapper,
    /// Kept alive for the case; it holds the OMOCL directory.
    _mappings: tempfile::TempDir,
}

/// Returns the canonical laboratory composition, built from its FLAT fixture.
fn laboratory_composition() -> Result<serde_json::Value, Box<dyn Error>> {
    use ferrobridge_testkit::fixtures::{LABORATORY_REPORT_FLAT, LABORATORY_REPORT_OPT};
    let source = openehr_mapping_core::template::TemplateSource::opt14(LABORATORY_REPORT_OPT)?;
    let index = openehr_mapping_core::index::WebTemplateIndex::build(&source)?;
    let flat: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(LABORATORY_REPORT_FLAT)?;
    Ok(index
        .build_from_flat(&flat, "2026-09-25T00:00:00Z")?
        .into_value())
}

/// Copies the laboratory corpus files into a directory of their own.
fn laboratory_mappings() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let mappings = tempfile::tempdir()?;
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/specs/omocl");
    for file in LABORATORY_FILES {
        let from = std::path::Path::new(corpus).join(file);
        let name = from.file_name().ok_or("a corpus file name")?;
        std::fs::copy(&from, mappings.path().join(name))?;
    }
    Ok(mappings)
}

/// Starts the laboratory harness.
async fn laboratory_harness() -> Result<LaboratoryHarness, Box<dyn Error>> {
    let postgres = containers::postgres().await?;
    let pool = CdmPool::connect(
        sqlx::postgres::PgPoolOptions::new().max_connections(2),
        postgres.url().parse()?,
        SchemaName::new("cdm")?,
    )
    .await?;
    database::init(&pool).await?;
    let mut connection = pool.pool().acquire().await?;
    ferrobridge_testkit::vocabulary::load(&mut connection).await?;
    drop(connection);
    let mut writer = CdmWriter::connect(
        postgres.url(),
        SchemaName::new("cdm")?,
        SchemaName::new("ferrobridge")?,
        PersonPolicy::CreateOnFirstSight,
    )
    .await?;
    writer.init().await?;

    let mappings = laboratory_mappings()?;
    let mapper = ferrobridge_server::etl::mapper::OmoclMapper::new(
        ferrobridge_server::etl::mapper::read_set(mappings.path())?,
        ferrobridge_server::etl::mapper::CdmVocabulary::new(
            omop_cdm::vocabulary::ConceptResolver::new(pool),
        ),
        omocl::engine::concept::VocabularyAliases::default(),
    );

    let composition = laboratory_composition()?;
    let rows: Vec<serde_json::Value> = COMPOSITIONS_UIDS
        .iter()
        .take(2)
        .map(|uid| {
            json!([
                EHR,
                uid,
                format!("{uid}::ferrobridge.test::1"),
                composition.clone()
            ])
        })
        .collect();
    let cdr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/query/aql"))
        .and(body_string_contains("AS composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "rows": rows })))
        .mount(&cdr)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/definition/template/adl1.4/ferrobridge.laboratory_report.v1",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(ferrobridge_testkit::fixtures::LABORATORY_REPORT_OPT),
        )
        .expect(1..)
        .mount(&cdr)
        .await;
    Ok(LaboratoryHarness {
        postgres,
        cdr,
        writer,
        mapper,
        _mappings: mappings,
    })
}

/// Runs the ETL once through the OMOCL engine.
async fn engine_run(
    harness: &mut LaboratoryHarness,
    resume: bool,
    run_id: &str,
) -> Result<RunReport, Box<dyn Error>> {
    let options = RunOptions {
        resume,
        since: None,
    };
    Ok(run(
        &settings(false)?,
        &options,
        &client(&harness.cdr)?,
        &mut harness.writer,
        &harness.mapper,
        &RunId::new(run_id)?,
    )
    .await?)
}

#[tokio::test]
async fn the_omocl_engine_writes_the_laboratory_compositions_and_a_rerun_changes_nothing()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut harness = laboratory_harness().await?;
    let first = engine_run(&mut harness, false, "run-1").await?;
    assert_eq!(2, first.compositions.committed, "{first}");
    assert!(first.refusals.is_empty(), "{first}");
    assert_eq!(Some(&4), first.rows.get("measurement"), "{first}");
    assert_eq!(Some(&2), first.rows.get("specimen"), "{first}");
    assert_eq!(
        8, first.zero_relationship_links,
        "each analyte links to its specimen, both ways"
    );
    assert_eq!(
        4,
        count(
            &harness.postgres,
            "SELECT count(*) FROM cdm.measurement WHERE measurement_type_concept_id = 32817"
        )
        .await?
    );
    assert_eq!(
        8,
        count(
            &harness.postgres,
            "SELECT count(*) FROM cdm.fact_relationship"
        )
        .await?
    );
    let before = dump(&harness.postgres)?;

    let second = engine_run(&mut harness, false, "run-2").await?;
    assert_eq!(2, second.compositions.committed);
    assert_eq!(
        before,
        dump(&harness.postgres)?,
        "a re-run through the engine leaves the data as it was"
    );

    let resumed = engine_run(&mut harness, true, "run-3").await?;
    assert_eq!(2, resumed.compositions.skipped);
    assert_eq!(before, dump(&harness.postgres)?);
    Ok(())
}

#[test]
fn the_laboratory_mappings_load_for_the_engine() -> Result<(), Box<dyn Error>> {
    let mappings = laboratory_mappings()?;
    let set = ferrobridge_server::etl::mapper::read_set(mappings.path())?;
    assert_eq!(3, set.len());
    Ok(())
}
