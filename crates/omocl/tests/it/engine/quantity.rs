// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How a quantity projects its range, its operator and its date.

use std::error::Error;

use omocl::engine::RefusalKind;
use serde_json::json;

use crate::lab::FIRST_ANALYTE_NODE;
use crate::lab::StubSource;
use crate::lab::composition;
use crate::lab::edited;
use crate::lab::flat;
use crate::lab::template;

use crate::engine::cell;
use crate::engine::float;
use crate::engine::int;
use crate::engine::measurements;
use crate::engine::outcome;
use crate::engine::ranged;
use crate::engine::refused;
use crate::engine::text;

#[tokio::test]
async fn a_quantity_projects_its_range_and_its_operator() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = ranged(&index, "<", "mmol/L")?;
    let (graph, refusals) = outcome(&[], &index, &composition, &stub).await?;
    assert!(refusals.is_empty(), "{refusals:?}");
    let row = measurements(&graph).into_iter().next().expect("a row");
    assert_eq!(float(row, "range_low"), Some(3.5));
    assert_eq!(float(row, "range_high"), Some(7.0));
    assert_eq!(int(row, "operator_concept_id"), Some(5_001));
    assert_eq!(int(row, "unit_concept_id"), Some(3_001));
    assert_eq!(text(row, "unit_source_value"), Some("mmol/L"));
    Ok(())
}

#[tokio::test]
async fn an_exact_status_leaves_the_operator_null() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = ranged(&index, "=", "mmol/L")?;
    let (graph, _) = outcome(&[], &index, &composition, &stub).await?;
    let row = measurements(&graph).into_iter().next().expect("a row");
    assert_eq!(row.cell("operator_concept_id"), None);
    Ok(())
}

#[tokio::test]
async fn a_status_the_cdm_has_no_operator_for_writes_concept_zero() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = ranged(&index, "~", "mmol/L")?;
    let (graph, _) = outcome(&[], &index, &composition, &stub).await?;
    let row = measurements(&graph).into_iter().next().expect("a row");
    assert_eq!(int(row, "operator_concept_id"), Some(0));
    Ok(())
}

#[tokio::test]
async fn a_range_in_another_unit_refuses_the_record() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = ranged(&index, "=", "mg/dL")?;
    let (_, refusals) = outcome(&[], &index, &composition, &stub).await?;
    assert_eq!(refused(&refusals, RefusalKind::RangeUnit), 1);
    Ok(())
}

#[tokio::test]
async fn a_date_without_a_time_leaves_the_datetime_null() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let built = composition(&index, &flat(&[], &[])?)?;
    let composition = edited(&built, |value| {
        if let Some(time) = value.pointer_mut(&format!("{FIRST_ANALYTE_NODE}/items/2/value/value"))
        {
            *time = json!("2026-09-19");
        }
    });
    let (graph, _) = outcome(&[], &index, &composition, &stub).await?;
    let row = measurements(&graph).into_iter().next().expect("a row");
    assert_eq!(
        row.cell("measurement_date").map(cell),
        Some("2026-09-19".to_owned())
    );
    assert_eq!(row.cell("measurement_datetime"), None);
    Ok(())
}
