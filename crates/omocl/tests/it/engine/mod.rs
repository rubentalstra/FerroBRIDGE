// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The engine over the synthetic laboratory composition: each rule the
//! engine fills an OMOCL silence with, in isolation, and the two corpus
//! laboratory files end to end as a snapshot.

mod columns;
mod forms;
mod laboratory;
mod quantity;
mod run_outcome;
mod vocabulary;

use std::error::Error;
use std::fmt::Write as _;

use omocl::engine::RecordRefusal;
use omocl::engine::RefusalKind;
use omocl::engine::concept::VocabularyAliases;
use omocl::engine::run;
use omop_cdm::graph::RecordGraph;
use omop_cdm::graph::row::Cell;
use omop_cdm::graph::row::Reference;
use omop_cdm::graph::row::Row;
use omop_cdm::graph::row::Value;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use serde_json::json;

use crate::lab::LABORATORY;
use crate::lab::StubSource;
use crate::lab::compiled;
use crate::lab::composition;
use crate::lab::edited;
use crate::lab::first_quantity;
use crate::lab::flat;
use crate::lab::seams;
use crate::lab::set;
use crate::lab::source;
use crate::lab::template;
use crate::lab::visit;
use crate::resolve::ANALYTE;
use crate::resolve::mapping;

/// The instance path of the first analyte.
const FIRST_ANALYTE: &str = "/content[openEHR-EHR-OBSERVATION.laboratory_test_result.v1 and \
                             1]/data[at0001 and 1]/events[at0002 and 1]/data[at0003 and \
                             1]/items[openEHR-EHR-CLUSTER.laboratory_test_analyte.v1 and 1]";

/// Runs `inline` (or the laboratory corpus files when `inline` is empty) over
/// `composition` against `stub`.
async fn outcome(
    inline: &[(&str, &str)],
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    stub: &StubSource,
) -> Result<(RecordGraph, Vec<RecordRefusal>), Box<dyn Error>> {
    let corpus: &[&str] = if inline.is_empty() { LABORATORY } else { &[] };
    let program = compiled(&set(corpus, inline)?, index)?;
    let aliases = VocabularyAliases::default();
    let visit = visit()?;
    let outcome = run(
        &program,
        composition,
        index,
        &source()?,
        Some(&visit),
        &seams(stub, &aliases),
    )
    .await?;
    Ok(outcome.into_parts())
}

/// Runs the laboratory corpus files over the fixture composition.
async fn laboratory(
    stub: &StubSource,
) -> Result<(RecordGraph, Vec<RecordRefusal>), Box<dyn Error>> {
    let index = template()?;
    let composition = composition(&index, &flat(&[], &[])?)?;
    outcome(&[], &index, &composition, stub).await
}

/// Returns the rows of `table`.
fn rows<'g>(graph: &'g RecordGraph, table: &str) -> Vec<&'g Row> {
    graph
        .rows()
        .iter()
        .filter(|row| row.table().name == table)
        .collect()
}

/// Returns the `MEASUREMENT` rows of a graph.
fn measurements(graph: &RecordGraph) -> Vec<&Row> {
    rows(graph, "measurement")
}

/// Returns an integer cell.
fn int(row: &Row, column: &str) -> Option<i32> {
    match row.cell(column) {
        Some(Cell::Value(Value::Integer(value))) => Some(*value),
        _ => None,
    }
}

/// Returns a float cell.
fn float(row: &Row, column: &str) -> Option<f64> {
    match row.cell(column) {
        Some(Cell::Value(Value::Float(value))) => Some(*value),
        _ => None,
    }
}

/// Returns a text cell.
fn text<'r>(row: &'r Row, column: &str) -> Option<&'r str> {
    match row.cell(column) {
        Some(Cell::Value(Value::Text(value))) => Some(value),
        _ => None,
    }
}

/// Renders a cell for the snapshot.
fn cell(cell: &Cell) -> String {
    match cell {
        Cell::Value(Value::Integer(value)) => value.to_string(),
        Cell::Value(Value::Float(value)) => format!("{value:?}"),
        Cell::Value(Value::Text(value)) => format!("{value:?}"),
        Cell::Value(Value::Date(value)) => value.to_string(),
        Cell::Value(Value::Datetime(value)) => value.to_string(),
        Cell::Reference(Reference::Person(ehr)) => format!("person {}", ehr.value()),
        Cell::Reference(Reference::Visit(visit)) => format!("visit {}", visit.source()),
        Cell::Reference(Reference::Row(key)) => format!("row {key}"),
    }
}

/// Renders a graph as the snapshot pins it.
fn render(graph: &RecordGraph) -> Result<String, Box<dyn Error>> {
    let mut out = String::new();
    for row in graph.rows() {
        writeln!(
            out,
            "{} {} ({})",
            row.table().name,
            row.key(),
            row.mapping()
        )?;
        for (column, value) in row.cells() {
            writeln!(out, "  {column} = {}", cell(value))?;
        }
    }
    for link in graph.links() {
        writeln!(
            out,
            "link {} {} [{}] <-> {} {} [{}]",
            link.first().table().name,
            link.first().key(),
            link.first().domain_concept_id(),
            link.second().table().name,
            link.second().key(),
            link.second().domain_concept_id()
        )?;
    }
    let report = graph.report();
    for refusal in report.refusals() {
        writeln!(out, "refused {}", refusal.reason())?;
    }
    for field in report.unmapped_fields() {
        writeln!(out, "unmapped {} {}", field.mapping(), field.element())?;
    }
    Ok(out)
}

/// Returns an analyte mapping with `columns` under one `Measurement`.
fn analyte(name: &str, columns: &str) -> String {
    mapping(
        name,
        ANALYTE,
        &format!("  - type: \"Measurement\"\n{columns}"),
    )
}

/// The two columns every analyte mapping below needs.
const CONCEPT_AND_DATE: &str = "    concept_id:\n      alternatives:\n        - path: \
                                \"/items[at0024]\"\n    measurement_date:\n      \
                                alternatives:\n        - path: \"/items[at0025]\"\n        - \
                                path: \"../../\"\n";

/// One fanned-out row: its concept, source concept, source value and branch.
type Branch<'r> = (Option<i32>, Option<i32>, Option<&'r str>, u16);

/// Returns how many refusals are of `kind`.
fn refused(refusals: &[RecordRefusal], kind: RefusalKind) -> usize {
    refusals
        .iter()
        .filter(|refusal| refusal.kind == kind)
        .count()
}

/// Returns the fixture composition with a normal range and a status on the
/// first analyte's quantity.
fn ranged(
    index: &WebTemplateIndex,
    status: &str,
    range_units: &str,
) -> Result<CanonicalComposition, Box<dyn Error>> {
    let built = composition(index, &flat(&[], &[])?)?;
    Ok(edited(&built, |value| {
        let Some(quantity) = first_quantity(value) else {
            return;
        };
        quantity["magnitude_status"] = json!(status);
        quantity["normal_range"] = json!({
            "_type": "DV_INTERVAL",
            "lower": {"_type": "DV_QUANTITY", "magnitude": 3.5, "units": range_units},
            "upper": {"_type": "DV_QUANTITY", "magnitude": 7.0, "units": range_units},
            "lower_unbounded": false,
            "upper_unbounded": false,
            "lower_included": true,
            "upper_included": true
        });
    }))
}
