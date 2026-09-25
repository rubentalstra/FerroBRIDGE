// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ETL run end to end over a stubbed CDR and a PostgreSQL CDM: the
//! composition query paged through ITS-REST 1.1.0 `POST /query/aql`, the
//! template read once, each composition mapped by a stub mapper that builds
//! the laboratory record graph by hand, the writer, the visits and the
//! derived tables. The rules are FerroBRIDGE's own (`docs/architecture.md`
//! §5.1). The container-backed cases run only when `FERROBRIDGE_E2E=1`.

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::config::Config;
use ferrobridge_server::etl::aql::CheckedQuery;
use ferrobridge_server::etl::report::RunReport;
use ferrobridge_server::etl::{
    EtlSettings, Mapper, RunOptions, SourceComposition, VisitSettings, run,
};
use ferrobridge_testkit::containers::{self, Postgres};
use ferrobridge_testkit::fixtures::{MINIMAL_EVALUATION_COMPOSITION, MINIMAL_EVALUATION_OPT};
use omop_cdm::database::{self, CdmPool};
use omop_cdm::ddl::SchemaName;
use omop_cdm::graph::{
    ArchetypeRootPath, Discriminator, Link, LinkEnd, MappingName, OccurrencePath, RecordGraph,
    RecordKey, Reference, Refusal, Row, Value, VisitKey, VisitSource,
};
use omop_cdm::value::CdmDate;
use omop_cdm::writer::{CdmWriter, PersonPolicy, RunId, VisitConcepts};
use serde_json::json;
use std::error::Error;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The composition query every case runs.
const COMPOSITIONS: &str = "SELECT e/ehr_id/value AS ehr_id, \
    vo/uid/value AS versioned_object_uid, v/uid/value AS version_uid, c AS composition \
    FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v CONTAINS COMPOSITION c \
    ORDER BY v/uid/value";

/// The visit query of the derivation case.
const VISITS: &str = "SELECT e/ehr_id/value AS ehr_id, c/name/value AS visit_source, \
    c/context/start_time/value AS visit_start, c/context/end_time/value AS visit_end \
    FROM EHR e CONTAINS COMPOSITION c ORDER BY c/context/start_time/value";

/// The synthetic EHR.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// The three synthetic versioned compositions.
const COMPOSITIONS_UIDS: [&str; 3] = [
    "8849182c-82ad-4088-a07f-48ead4180515",
    "1f7f3c1a-2b64-4a3c-9d0e-4a2f5c7b9e01",
    "5b0e9c3d-7a21-4c55-8e6f-0c9d8b7a6f52",
];

/// The mapping the stub mapper reports its rows under.
const MAPPING: &str = "Laboratory_test_result_v1";

/// The archetype root of the synthetic laboratory result.
const ROOT: &str = "/content[openEHR-EHR-OBSERVATION.laboratory_test_result.v1]";

/// The `domain_concept_id` the stub writes for a measurement end.
const MEASUREMENT_DOMAIN: i32 = 9011;

/// What the stub mapper does with one composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Two analytes, both mapped.
    Map,
    /// The converter refuses the second analyte, so the composition is refused.
    RefuseSecondAnalyte,
    /// The second analyte carries no date, so that record is refused.
    DropSecondDate,
}

/// A mapper that builds the laboratory graph by hand.
struct Laboratory {
    /// The behaviour for each versioned composition, by index.
    behaviours: [Behaviour; 3],
    /// Whether each analyte names the visit `encounter-1`.
    visit: bool,
}

impl Laboratory {
    /// Returns the key of the node at `occurrence` of `composition`.
    fn key(composition: &SourceComposition<'_>, occurrence: &str) -> RecordKey {
        RecordKey::new(
            composition.source.ehr_id().clone(),
            composition.source.versioned_object_uid().clone(),
            ArchetypeRootPath::new(ROOT).expect("a root path"),
            OccurrencePath::new(occurrence).expect("an occurrence path"),
            Discriminator::new(MappingName::new(MAPPING).expect("a name"), 0, 0),
        )
    }

    /// Returns a measurement row under `key`, dated `date` when given.
    fn row(
        &self,
        composition: &SourceComposition<'_>,
        key: RecordKey,
        date: Option<&str>,
    ) -> Result<Row, omop_cdm::graph::GraphError> {
        let mut builder = Row::builder("measurement", key)?
            .reference(
                "person_id",
                Reference::Person(composition.source.ehr_id().clone()),
            )?
            .value("measurement_concept_id", Value::Integer(1001))?
            .value(
                "measurement_type_concept_id",
                Value::Integer(composition.context.type_concept_id),
            )?
            .value("value_as_number", Value::Float(4.5))?
            .optional(
                "measurement_date",
                date.map(|date| Value::Date(CdmDate::new(date).expect("a date"))),
            )?;
        if self.visit {
            builder = builder.reference(
                "visit_occurrence_id",
                Reference::Visit(VisitKey::new(
                    composition.source.ehr_id().clone(),
                    VisitSource::new("encounter-1").expect("a source"),
                )),
            )?;
        }
        builder.build()
    }
}

impl Mapper for Laboratory {
    fn map(
        &self,
        composition: &SourceComposition<'_>,
    ) -> impl Future<Output = Result<RecordGraph, Refusal>> {
        std::future::ready(self.graph(composition))
    }
}

impl Laboratory {
    /// Builds the graph of `composition`.
    fn graph(&self, composition: &SourceComposition<'_>) -> Result<RecordGraph, Refusal> {
        let index = COMPOSITIONS_UIDS
            .iter()
            .position(|uid| *uid == composition.source.versioned_object_uid().as_str())
            .ok_or_else(|| Refusal::new("an unknown composition"))?;
        let behaviour = self
            .behaviours
            .get(index)
            .copied()
            .unwrap_or(Behaviour::Map);
        let refused = |error: omop_cdm::graph::GraphError| Refusal::of_row(&error);
        let mut graph = RecordGraph::new(composition.source.clone());
        let result = Self::key(composition, "/");
        graph
            .push_row(
                self.row(composition, result.clone(), Some("2026-06-15"))
                    .map_err(refused)?,
            )
            .map_err(refused)?;
        for analyte in 1..=2 {
            let occurrence =
                format!("/items[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1,{analyte}]");
            let date = if analyte == 2 && behaviour == Behaviour::DropSecondDate {
                None
            } else {
                Some("2026-06-15")
            };
            if analyte == 2 && behaviour == Behaviour::RefuseSecondAnalyte {
                return Err(Refusal::new("the converter refused the second analyte")
                    .with_mapping(MappingName::new(MAPPING).expect("a name"))
                    .with_element(occurrence));
            }
            let key = Self::key(composition, &occurrence);
            match self.row(composition, key.clone(), date) {
                Ok(row) => {
                    graph.push_row(row).map_err(refused)?;
                    graph
                        .push_link(Link::new(
                            LinkEnd::new("measurement", result.clone(), MEASUREMENT_DOMAIN)
                                .map_err(refused)?,
                            LinkEnd::new("measurement", key, MEASUREMENT_DOMAIN)
                                .map_err(refused)?,
                        ))
                        .map_err(refused)?;
                }
                Err(error) => graph.report_mut().refuse(
                    Refusal::of_row(&error)
                        .with_mapping(MappingName::new(MAPPING).expect("a name"))
                        .with_element(format!("{occurrence}/time")),
                ),
            }
        }
        Ok(graph)
    }
}

/// A started CDM database and a stubbed CDR serving three compositions.
struct Harness {
    /// Kept alive for the case; dropping it stops the container.
    postgres: Postgres,
    cdr: MockServer,
    writer: CdmWriter,
}

/// Returns the composition row for the composition at `index`.
fn composition_row(index: usize) -> serde_json::Value {
    let uid = COMPOSITIONS_UIDS[index];
    let composition: serde_json::Value =
        serde_json::from_str(MINIMAL_EVALUATION_COMPOSITION).expect("the fixture parses");
    json!([EHR, uid, format!("{uid}::ferrobridge.test::1"), composition])
}

/// Starts PostgreSQL with the CDM and bridge schemas, and a CDR stub
/// serving the three compositions and two rows of one visit.
async fn harness() -> Result<Harness, Box<dyn Error>> {
    harness_with(
        (0..3).map(composition_row).collect(),
        vec![
            json!([
                EHR,
                "encounter-1",
                "2026-06-14T09:00:00+02:00",
                "2026-06-14T10:00:00+02:00"
            ]),
            json!([
                EHR,
                "encounter-1",
                "2026-06-13T08:00:00+02:00",
                "2026-06-16T12:00:00+02:00"
            ]),
        ],
    )
    .await
}

/// Starts PostgreSQL with the CDM and bridge schemas, and a CDR stub
/// serving `compositions` and `visits` as the rows of the two queries.
async fn harness_with(
    compositions: Vec<serde_json::Value>,
    visits: Vec<serde_json::Value>,
) -> Result<Harness, Box<dyn Error>> {
    let postgres = containers::postgres().await?;
    let pool = CdmPool::connect(
        sqlx::postgres::PgPoolOptions::new().max_connections(1),
        postgres.url().parse()?,
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

    let cdr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/query/aql"))
        .and(body_string_contains("AS composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "rows": compositions })))
        .mount(&cdr)
        .await;
    Mock::given(method("POST"))
        .and(path("/query/aql"))
        .and(body_string_contains("AS visit_source"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "rows": visits })))
        .mount(&cdr)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/definition/template/adl1.4/ferrobridge.minimal_evaluation.v1",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(MINIMAL_EVALUATION_OPT),
        )
        .expect(1..)
        .mount(&cdr)
        .await;
    Ok(Harness {
        postgres,
        cdr,
        writer,
    })
}

/// Returns the CDR client over the stub.
fn client(cdr: &MockServer) -> Result<Client, Box<dyn Error>> {
    Ok(Client::new(Config::new(
        format!("{}/", cdr.uri()).parse()?,
    ))?)
}

/// Returns the settings of a run, with the visit derivation when `visits`.
fn settings(visits: bool) -> Result<EtlSettings, Box<dyn Error>> {
    Ok(EtlSettings {
        compositions: CheckedQuery::compositions(COMPOSITIONS)?,
        page_size: ferrobridge_openehr::query::PageSize::new(100)?,
        type_concept_id: 32817,
        observation_period_type_concept_id: 32880,
        visits: if visits {
            Some(VisitSettings {
                query: CheckedQuery::visits(VISITS)?,
                concepts: VisitConcepts {
                    visit_concept_id: 1001,
                    visit_type_concept_id: 32817,
                },
            })
        } else {
            None
        },
    })
}

/// Runs the ETL once with `mapper`.
async fn run_once(
    harness: &mut Harness,
    mapper: &Laboratory,
    visits: bool,
    resume: bool,
    run_id: &str,
) -> Result<RunReport, Box<dyn Error>> {
    let options = RunOptions {
        resume,
        since: None,
    };
    Ok(run(
        &settings(visits)?,
        &options,
        &client(&harness.cdr)?,
        &mut harness.writer,
        mapper,
        &RunId::new(run_id)?,
    )
    .await?)
}

/// Returns the data of the CDM and bridge schemas as `pg_dump` writes it,
/// with each table's rows sorted and the watermark's run columns cut, since
/// a re-run records its own run id and commit time.
pub(crate) fn dump(postgres: &Postgres) -> Result<String, Box<dyn Error>> {
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            postgres.container().id(),
            "pg_dump",
            "--username=ferrobridge",
            "--data-only",
            "--schema=cdm",
            "--schema=ferrobridge",
            "ferrobridge",
        ])
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let text = String::from_utf8(output.stdout)?;
    let mut out = Vec::new();
    let mut block: Option<(bool, Vec<String>)> = None;
    for line in text.lines() {
        if let Some((watermark, rows)) = block.as_mut() {
            if line == "\\." {
                rows.sort();
                out.append(rows);
                out.push(line.to_owned());
                block = None;
            } else if *watermark {
                let kept: Vec<&str> = line.split('\t').take(3).collect();
                rows.push(kept.join("\t"));
            } else {
                rows.push(line.to_owned());
            }
            continue;
        }
        if line.starts_with("COPY ") {
            block = Some((line.starts_with("COPY ferrobridge.watermark "), Vec::new()));
        }
        // NOTE: PostgreSQL docs, pg_dump `--restrict-key`: the `\restrict` and
        // `\unrestrict` lines carry a key drawn at random for every dump.
        let session = line.starts_with("\\restrict ") || line.starts_with("\\unrestrict ");
        if !session
            && !line.starts_with("--")
            && !line.starts_with("SET ")
            && !line.starts_with("SELECT pg_catalog.set_config")
        {
            out.push(line.to_owned());
        }
    }
    Ok(out.join("\n"))
}

/// Returns the number of rows `query` counts.
async fn count(postgres: &Postgres, query: &str) -> Result<i64, Box<dyn Error>> {
    let (client, connection) =
        tokio_postgres::connect(postgres.url(), tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    Ok(client.query_one(query, &[]).await?.get(0))
}

/// A mapper that maps every composition.
const MAP_ALL: Laboratory = Laboratory {
    behaviours: [Behaviour::Map; 3],
    visit: false,
};

#[tokio::test]
async fn a_run_writes_the_laboratory_rows_and_a_second_run_changes_nothing()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut harness = harness().await?;
    let first = run_once(&mut harness, &MAP_ALL, false, false, "run-1").await?;
    assert_eq!(3, first.compositions.committed, "{first}");
    assert_eq!(Some(&9), first.rows.get("measurement"));
    assert_eq!(
        12, first.zero_relationship_links,
        "two links per composition, both ways"
    );
    assert_eq!(1, first.derived.observation_period);
    assert_eq!(
        9,
        count(&harness.postgres, "SELECT count(*) FROM cdm.measurement").await?
    );
    assert_eq!(
        12,
        count(
            &harness.postgres,
            "SELECT count(*) FROM cdm.fact_relationship"
        )
        .await?
    );
    let before = dump(&harness.postgres)?;

    let second = run_once(&mut harness, &MAP_ALL, false, false, "run-2").await?;
    assert_eq!(3, second.compositions.committed);
    assert_eq!(
        before,
        dump(&harness.postgres)?,
        "a re-run leaves the data as it was"
    );

    let resumed = run_once(&mut harness, &MAP_ALL, false, true, "run-3").await?;
    assert_eq!(3, resumed.compositions.skipped);
    assert_eq!(0, resumed.compositions.committed);
    assert_eq!(before, dump(&harness.postgres)?);
    Ok(())
}

#[tokio::test]
async fn a_refused_composition_writes_nothing_and_a_resumed_run_finishes_it()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut harness = harness().await?;
    let refusing = Laboratory {
        behaviours: [
            Behaviour::Map,
            Behaviour::RefuseSecondAnalyte,
            Behaviour::Map,
        ],
        visit: false,
    };
    let first = run_once(&mut harness, &refusing, false, false, "run-1").await?;
    assert_eq!(2, first.compositions.committed);
    assert_eq!(1, first.compositions.refused);
    let refusal = first.refusals.first().ok_or("the refusal is reported")?;
    assert_eq!(Some(COMPOSITIONS_UIDS[1]), refusal.composition.as_deref());
    assert_eq!(
        0,
        count(
            &harness.postgres,
            &format!(
                "SELECT count(*) FROM ferrobridge.record WHERE versioned_object_uid = '{}'",
                COMPOSITIONS_UIDS[1]
            )
        )
        .await?,
        "no row of the refused composition was written"
    );
    assert_eq!(
        6,
        count(&harness.postgres, "SELECT count(*) FROM cdm.measurement").await?
    );
    assert_eq!(
        2,
        count(
            &harness.postgres,
            "SELECT count(*) FROM ferrobridge.watermark"
        )
        .await?
    );

    let resumed = run_once(&mut harness, &MAP_ALL, false, true, "run-2").await?;
    assert_eq!(2, resumed.compositions.skipped);
    assert_eq!(1, resumed.compositions.committed);
    assert_eq!(
        9,
        count(&harness.postgres, "SELECT count(*) FROM cdm.measurement").await?
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_required_date_refuses_the_record_and_names_the_element()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut harness = harness().await?;
    let dropping = Laboratory {
        behaviours: [Behaviour::DropSecondDate, Behaviour::Map, Behaviour::Map],
        visit: false,
    };
    let report = run_once(&mut harness, &dropping, false, false, "run-1").await?;
    assert_eq!(3, report.compositions.committed);
    let refusal = report
        .refusals
        .first()
        .ok_or("the record refusal is reported")?;
    assert_eq!(Some("measurement"), refusal.table.as_deref());
    assert_eq!(Some("measurement_date"), refusal.column.as_deref());
    assert_eq!(
        Some("/items[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1,2]/time"),
        refusal.element.as_deref()
    );
    assert_eq!(
        Some(&1),
        report.mappings.get(MAPPING).map(|totals| &totals.refusals)
    );
    assert_eq!(
        8,
        count(&harness.postgres, "SELECT count(*) FROM cdm.measurement").await?
    );
    let json = serde_json::to_value(&report)?;
    assert_eq!("measurement_date", json["refusals"][0]["column"]);
    assert!(report.to_string().contains("measurement_date"), "{report}");
    Ok(())
}

#[tokio::test]
async fn the_visits_and_the_derived_tables_are_populated() -> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut harness = harness().await?;
    let visiting = Laboratory {
        behaviours: [Behaviour::Map; 3],
        visit: true,
    };
    let report = run_once(&mut harness, &visiting, true, false, "run-1").await?;
    assert_eq!(1, report.derived.visit_occurrence);
    assert_eq!(1, report.derived.observation_period);
    assert_eq!(
        1,
        count(
            &harness.postgres,
            "SELECT count(*) FROM cdm.visit_occurrence
             WHERE visit_start_date = '2026-06-13' AND visit_end_date = '2026-06-16'
               AND visit_start_datetime = '2026-06-13T08:00:00'"
        )
        .await?,
        "the visit spans the earliest start to the latest end"
    );
    assert_eq!(
        9,
        count(
            &harness.postgres,
            "SELECT count(*) FROM cdm.measurement WHERE visit_occurrence_id IS NOT NULL"
        )
        .await?
    );
    assert_eq!(
        1,
        count(
            &harness.postgres,
            "SELECT count(*) FROM cdm.observation_period
             WHERE observation_period_start_date = '2026-06-13'
               AND observation_period_end_date = '2026-06-16'"
        )
        .await?
    );
    Ok(())
}

/// The versioned compositions of the visit tie cases, by the case each one
/// exercises.
const TIE_UIDS: [&str; 4] = [
    "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e1",
    "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e2",
    "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e3",
    "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e4",
];

/// A mapper that writes one measurement per composition, carrying the visit
/// the run tied the composition to.
struct Tied;

impl Mapper for Tied {
    fn map(
        &self,
        composition: &SourceComposition<'_>,
    ) -> impl Future<Output = Result<RecordGraph, Refusal>> {
        std::future::ready(Self::graph(composition))
    }
}

impl Tied {
    /// Builds the graph of `composition`.
    fn graph(composition: &SourceComposition<'_>) -> Result<RecordGraph, Refusal> {
        let refused = |error: omop_cdm::graph::GraphError| Refusal::of_row(&error);
        let mut builder = Row::builder("measurement", Laboratory::key(composition, "/"))
            .map_err(refused)?
            .reference(
                "person_id",
                Reference::Person(composition.source.ehr_id().clone()),
            )
            .map_err(refused)?
            .value("measurement_concept_id", Value::Integer(1001))
            .map_err(refused)?
            .value(
                "measurement_type_concept_id",
                Value::Integer(composition.context.type_concept_id),
            )
            .map_err(refused)?
            .value(
                "measurement_date",
                Value::Date(CdmDate::new("2026-06-14").expect("a date")),
            )
            .map_err(refused)?;
        if let Some(visit) = composition.visit {
            builder = builder
                .reference("visit_occurrence_id", Reference::Visit(visit.clone()))
                .map_err(refused)?;
        }
        let mut graph = RecordGraph::new(composition.source.clone());
        graph
            .push_row(builder.build().map_err(refused)?)
            .map_err(refused)?;
        Ok(graph)
    }
}

/// Returns the composition row of `uid`, its context starting at `start` at
/// the health care facility `facility` when one is given.
fn tie_row(uid: &str, start: &str, facility: Option<&str>) -> serde_json::Value {
    let mut composition: serde_json::Value =
        serde_json::from_str(MINIMAL_EVALUATION_COMPOSITION).expect("the fixture parses");
    composition["context"]["start_time"]["value"] = json!(start);
    if let Some(facility) = facility {
        composition["context"]["health_care_facility"] =
            json!({"_type": "PARTY_IDENTIFIED", "name": facility});
    }
    json!([EHR, uid, format!("{uid}::ferrobridge.test::1"), composition])
}

#[tokio::test]
async fn each_composition_is_tied_to_the_visit_whose_window_contains_it()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let mut harness = harness_with(
        vec![
            tie_row(TIE_UIDS[0], "2026-06-14T10:00:00+02:00", None),
            tie_row(TIE_UIDS[1], "2026-07-01T10:00:00+02:00", None),
            tie_row(TIE_UIDS[2], "2026-06-15T09:00:00+02:00", Some("ward-b")),
            tie_row(TIE_UIDS[3], "2026-06-15T10:00:00+02:00", None),
        ],
        vec![
            json!([
                EHR,
                "ward-a",
                "2026-06-13T08:00:00+02:00",
                "2026-06-16T12:00:00+02:00"
            ]),
            json!([
                EHR,
                "ward-b",
                "2026-06-15T08:00:00+02:00",
                "2026-06-15T18:00:00+02:00"
            ]),
        ],
    )
    .await?;
    let report = run(
        &settings(true)?,
        &RunOptions::default(),
        &client(&harness.cdr)?,
        &mut harness.writer,
        &Tied,
        &RunId::new("run-1")?,
    )
    .await?;
    assert_eq!(4, report.compositions.committed, "{report}");
    assert_eq!(2, report.compositions.with_visit, "{report}");
    assert_eq!(
        1, report.compositions.without_visit,
        "inside no visit: {report}"
    );
    assert_eq!(
        1, report.compositions.ambiguous_visit,
        "undecided: {report}"
    );
    assert!(
        report.to_string().contains("1 inside no visit"),
        "the text report counts it: {report}"
    );
    let (client, connection) =
        tokio_postgres::connect(harness.postgres.url(), tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let tied: Vec<(String, Option<String>)> = client
        .query(
            "SELECT r.versioned_object_uid, v.visit_source_value
             FROM ferrobridge.record r
             JOIN cdm.measurement m ON r.cdm_table = 'measurement' AND m.measurement_id = r.surrogate_id
             LEFT JOIN cdm.visit_occurrence v ON v.visit_occurrence_id = m.visit_occurrence_id
             ORDER BY r.versioned_object_uid",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        vec![
            (TIE_UIDS[0].to_owned(), Some(String::from("ward-a"))),
            (TIE_UIDS[1].to_owned(), None),
            (TIE_UIDS[2].to_owned(), Some(String::from("ward-b"))),
            (TIE_UIDS[3].to_owned(), None),
        ],
        tied,
        "inside one window, inside none, inside two with the facility deciding, inside two undecided"
    );
    Ok(())
}

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
