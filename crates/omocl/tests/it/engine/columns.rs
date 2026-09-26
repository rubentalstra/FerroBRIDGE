// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How a column picks its alternative, and how a path selects the nodes a
//! record reads.

use std::error::Error;

use omocl::engine::RefusalKind;

use crate::lab::EVENT;
use crate::lab::StubSource;
use crate::lab::composition;
use crate::lab::flat;
use crate::lab::template;
use crate::resolve::ANALYTE;
use crate::resolve::RESULT;
use crate::resolve::mapping;

use crate::engine::FIRST_ANALYTE;
use crate::engine::cell;
use crate::engine::float;
use crate::engine::laboratory;
use crate::engine::measurements;
use crate::engine::outcome;
use crate::engine::refused;

#[tokio::test]
async fn the_first_present_alternative_wins() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let (graph, _) = laboratory(&stub).await?;
    let dates: Vec<String> = measurements(&graph)
        .iter()
        .map(|row| {
            row.cell("measurement_datetime")
                .map(cell)
                .unwrap_or_default()
        })
        .collect();
    // The first analyte carries its own result time; the second falls back to
    // the event's time through `../../`.
    assert_eq!(dates, vec!["2026-09-20T09:15:00", "2026-09-20T08:00:00"]);
    Ok(())
}

#[tokio::test]
async fn a_required_column_with_no_present_alternative_refuses_the_record()
-> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(
        &index,
        &flat(&[], &[&format!("{EVENT}/synthetic_analyte:0/analyte_name")])?,
    )?;
    let (graph, refusals) = outcome(&[], &index, &composition, &stub).await?;
    assert_eq!(
        measurements(&graph).len(),
        1,
        "the other analyte still runs"
    );
    let refusal = refusals.first().expect("a refusal");
    assert_eq!(refusal.kind, RefusalKind::MissingColumn);
    assert_eq!(refusal.column, Some("concept_id"));
    assert_eq!(refusal.mapping, "Laboratory_test_analyte_v1");
    assert_eq!(refusal.path, FIRST_ANALYTE);
    assert_eq!(refusals.len(), 1);
    assert_eq!(
        graph.report().refusals().len(),
        1,
        "the report carries it too"
    );
    // The refused analyte has no row, so it has no link either.
    assert_eq!(graph.links().len(), 1);
    Ok(())
}

/// A result-level measurement whose value path reaches every analyte.
fn result_level(base: bool) -> String {
    let analytes = format!("data[at0003]/items[{ANALYTE}]");
    let (base_path, value, date) = if base {
        (
            format!("    base_path: \"/data[at0001]/events[at0002]/{analytes}\"\n"),
            "/items[at0001]".to_owned(),
            "../../".to_owned(),
        )
    } else {
        (
            String::new(),
            format!("/data[at0001]/events[at0002]/{analytes}/items[at0001]"),
            "/data[at0001]/events[at0002]".to_owned(),
        )
    };
    mapping(
        "Synthetic_result_v1",
        RESULT,
        &format!(
            "  - type: \"Measurement\"\n{base_path}    concept_id:\n      alternatives:\n        \
             - code: 1001\n    measurement_date:\n      alternatives:\n        - path: \
             \"{date}\"\n    value:\n      alternatives:\n        - path: \"{value}\"\n"
        ),
    )
}

#[tokio::test]
async fn a_path_matching_several_nodes_outside_an_iteration_refuses_the_record()
-> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(&index, &flat(&[], &[])?)?;
    let file = result_level(false);
    let (graph, refusals) = outcome(
        &[("Synthetic_result_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    assert!(graph.rows().is_empty());
    assert_eq!(refused(&refusals, RefusalKind::Multiple), 1);
    assert_eq!(refusals.first().and_then(|r| r.column), Some("value"));
    Ok(())
}

#[tokio::test]
async fn a_base_path_iterates_one_row_per_node() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(&index, &flat(&[], &[])?)?;
    let file = result_level(true);
    let (graph, refusals) = outcome(
        &[("Synthetic_result_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    let values: Vec<Option<f64>> = measurements(&graph)
        .iter()
        .map(|row| float(row, "value_as_number"))
        .collect();
    assert_eq!(values, vec![Some(7.25), Some(140.0)]);
    assert!(refusals.is_empty(), "{refusals:?}");
    Ok(())
}
