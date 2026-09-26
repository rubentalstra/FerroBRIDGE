// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two corpus laboratory files end to end, and what the engine fills in
//! around the rows they emit.

use std::error::Error;

use omop_cdm::graph::row::Cell;
use omop_cdm::graph::row::Reference;

use crate::lab::StubSource;
use crate::lab::TYPE_CONCEPT;
use crate::lab::source;
use crate::lab::visit;

use crate::engine::int;
use crate::engine::laboratory;
use crate::engine::measurements;
use crate::engine::render;

#[tokio::test]
async fn the_laboratory_files_emit_measurements_a_specimen_and_their_links()
-> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let (graph, refusals) = laboratory(&stub).await?;
    assert!(refusals.is_empty(), "{refusals:?}");
    insta::assert_snapshot!("laboratory_graph", render(&graph)?);
    Ok(())
}

#[tokio::test]
async fn the_engine_fills_the_type_concept_and_references_the_person_and_the_visit()
-> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let (graph, _) = laboratory(&stub).await?;
    let row = measurements(&graph).into_iter().next().expect("a row");
    assert_eq!(int(row, "measurement_type_concept_id"), Some(TYPE_CONCEPT));
    assert_eq!(
        row.cell("person_id"),
        Some(&Cell::Reference(Reference::Person(
            source()?.ehr_id().clone()
        )))
    );
    assert_eq!(
        row.cell("visit_occurrence_id"),
        Some(&Cell::Reference(Reference::Visit(visit()?)))
    );
    assert_eq!(
        row.cell("measurement_id"),
        None,
        "the writer assigns the id"
    );
    Ok(())
}
