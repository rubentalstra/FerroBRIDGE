// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The tie of each composition to the visit whose window contains it.

use ferrobridge_server::etl::{Mapper, RunOptions, SourceComposition, run};
use ferrobridge_testkit::containers::{self};
use ferrobridge_testkit::fixtures::MINIMAL_EVALUATION_COMPOSITION;
use omop_cdm::graph::{RecordGraph, report::Refusal, row::Reference, row::Row, row::Value};
use omop_cdm::value::CdmDate;
use omop_cdm::writer::input::RunId;
use serde_json::json;
use std::error::Error;

use super::EHR;
use super::Laboratory;
use super::client;
use super::harness_with;
use super::settings;

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
        let refused = |error: omop_cdm::graph::row::GraphError| Refusal::of_row(&error);
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
