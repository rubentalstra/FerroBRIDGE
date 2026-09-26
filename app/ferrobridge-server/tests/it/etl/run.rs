// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The run over the stub mapper: a rerun that changes nothing, a refusal and
//! its resume, a missing required date, the visits and the derived tables.

use ferrobridge_server::etl::report::RunReport;
use ferrobridge_server::etl::{RunOptions, run};
use ferrobridge_testkit::containers::{self};
use omop_cdm::writer::input::RunId;
use std::error::Error;

use super::Behaviour;
use super::COMPOSITIONS_UIDS;
use super::Harness;
use super::Laboratory;
use super::MAPPING;
use super::client;
use super::count;
use super::dump;
use super::harness;
use super::settings;

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
