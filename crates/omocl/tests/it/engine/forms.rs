// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `conceptMap` and `multiplication` alternative forms.

use std::error::Error;

use omocl::engine::RefusalKind;
use serde_json::json;

use crate::lab::EVENT;
use crate::lab::StubSource;
use crate::lab::composition;
use crate::lab::edited;
use crate::lab::first_quantity;
use crate::lab::flat;
use crate::lab::template;

use crate::engine::CONCEPT_AND_DATE;
use crate::engine::analyte;
use crate::engine::float;
use crate::engine::int;
use crate::engine::measurements;
use crate::engine::outcome;
use crate::engine::refused;
use crate::engine::text;

/// An analyte mapping whose value is a `conceptMap` over the analyte name.
fn concept_mapped() -> String {
    analyte(
        "Synthetic_map_v1",
        &format!(
            "{CONCEPT_AND_DATE}    value:\n      alternatives:\n        - conceptMap:\n            \
             path: \"/items[at0024]\"\n            mapping:\n              at0005: 1003\n"
        ),
    )
}

#[tokio::test]
async fn a_concept_map_writes_the_concept_its_at_code_lists() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let name = format!("{EVENT}/synthetic_analyte:0/analyte_name");
    let composition = composition(
        &index,
        &flat(
            &[
                (&format!("{name}|code"), json!("at0005")),
                (&format!("{name}|terminology"), json!("local")),
            ],
            &[&format!("{EVENT}/synthetic_analyte:1")],
        )?,
    )?;
    let file = concept_mapped();
    let (graph, _) = outcome(
        &[("Synthetic_map_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    let row = measurements(&graph).into_iter().next().expect("a row");
    assert_eq!(int(row, "value_as_concept_id"), Some(1_003));
    assert_eq!(text(row, "value_source_value"), Some("at0005"));
    // The local at-code names no vocabulary concept, so the primary concept
    // is concept 0.
    assert_eq!(int(row, "measurement_concept_id"), Some(0));
    Ok(())
}

#[tokio::test]
async fn a_concept_map_refuses_an_at_code_it_does_not_list() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(&index, &flat(&[], &[])?)?;
    let file = concept_mapped();
    let (graph, refusals) = outcome(
        &[("Synthetic_map_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    assert!(graph.rows().is_empty());
    assert_eq!(refused(&refusals, RefusalKind::UnlistedAtCode), 2);
    Ok(())
}

/// An analyte mapping whose value is the result times a literal factor.
fn multiplied(factor: i64) -> String {
    analyte(
        "Synthetic_product_v1",
        &format!(
            "{CONCEPT_AND_DATE}    value:\n      alternatives:\n        - multiplication:\n            \
             - path: \"/items[at0001]\"\n            - code: {factor}\n"
        ),
    )
}

#[tokio::test]
async fn a_multiplication_is_the_exact_product_of_its_factors() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(&index, &flat(&[], &[])?)?;
    let file = multiplied(4);
    let (graph, _) = outcome(
        &[("Synthetic_product_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    let values: Vec<Option<f64>> = measurements(&graph)
        .iter()
        .map(|row| float(row, "value_as_number"))
        .collect();
    assert_eq!(values, vec![Some(29.0), Some(560.0)]);
    Ok(())
}

#[tokio::test]
async fn a_multiplication_that_overflows_refuses_the_record() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let built = composition(&index, &flat(&[], &[])?)?;
    let composition = edited(&built, |value| {
        if let Some(quantity) = first_quantity(value) {
            quantity["magnitude"] = json!(1e28);
        }
    });
    let file = multiplied(10);
    let (graph, refusals) = outcome(
        &[("Synthetic_product_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    assert_eq!(measurements(&graph).len(), 1);
    assert_eq!(refused(&refusals, RefusalKind::Overflow), 1);
    Ok(())
}
