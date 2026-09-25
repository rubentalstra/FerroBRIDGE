// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The engine over the synthetic laboratory composition: each rule the
//! engine fills an OMOCL silence with, in isolation, and the two corpus
//! laboratory files end to end as a snapshot.

use std::error::Error;
use std::fmt::Write as _;

use omocl::engine::EngineError;
use omocl::engine::RecordRefusal;
use omocl::engine::RefusalKind;
use omocl::engine::concept::VocabularyAliases;
use omocl::engine::run;
use omop_cdm::graph::Cell;
use omop_cdm::graph::RecordGraph;
use omop_cdm::graph::Reference;
use omop_cdm::graph::Row;
use omop_cdm::graph::Value;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;
use serde_json::json;

use crate::lab::EVENT;
use crate::lab::FIRST_ANALYTE_NODE;
use crate::lab::LABORATORY;
use crate::lab::StubSource;
use crate::lab::Stubbed;
use crate::lab::TYPE_CONCEPT;
use crate::lab::compiled;
use crate::lab::composition;
use crate::lab::concept;
use crate::lab::edited;
use crate::lab::first_quantity;
use crate::lab::flat;
use crate::lab::mapped;
use crate::lab::seams;
use crate::lab::set;
use crate::lab::source;
use crate::lab::standard;
use crate::lab::template;
use crate::lab::visit;
use crate::resolve::ANALYTE;
use crate::resolve::RESULT;
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
        Cell::Reference(Reference::Person(ehr)) => format!("person {ehr}"),
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

#[tokio::test]
async fn a_concept_outside_the_domain_of_its_type_refuses_the_record() -> Result<(), Box<dyn Error>>
{
    let mut stub = StubSource::laboratory()?;
    stub.code(
        "LOINC",
        "SYN-LAB-1",
        standard(concept(1_101, "Condition", "LOINC", "SYN-LAB-1", true)?),
    );
    let (graph, refusals) = laboratory(&stub).await?;
    assert_eq!(measurements(&graph).len(), 1);
    assert_eq!(refused(&refusals, RefusalKind::DomainMismatch), 1);
    assert_eq!(graph.report().refusals().len(), 1, "the refusal is counted");
    let refusal = refusals.first().expect("a refusal");
    assert_eq!(refusal.column, Some("concept_id"));
    assert!(
        refusal.message.contains("`Condition` domain"),
        "{}",
        refusal.message
    );
    Ok(())
}

#[tokio::test]
async fn a_standard_and_a_mapped_code_write_their_standard_and_source_concepts()
-> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let (graph, _) = laboratory(&stub).await?;
    let concepts: Vec<(Option<i32>, Option<i32>)> = measurements(&graph)
        .iter()
        .map(|row| {
            (
                int(row, "measurement_concept_id"),
                int(row, "measurement_source_concept_id"),
            )
        })
        .collect();
    assert_eq!(
        concepts,
        vec![(Some(1_001), Some(1_001)), (Some(1_002), Some(2_001))]
    );
    Ok(())
}

#[tokio::test]
async fn an_unmapped_unit_writes_concept_zero_and_keeps_its_source() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let (graph, _) = laboratory(&stub).await?;
    let second = measurements(&graph)
        .into_iter()
        .nth(1)
        .expect("a second row");
    assert_eq!(int(second, "unit_concept_id"), Some(0));
    assert_eq!(text(second, "unit_source_value"), Some("g/L"));
    Ok(())
}

#[tokio::test]
async fn an_ambiguous_code_refuses_the_record() -> Result<(), Box<dyn Error>> {
    let mut stub = StubSource::laboratory()?;
    stub.code("LOINC", "SYN-LAB-2", Stubbed::Ambiguous(vec![2_006, 2_007]));
    let (graph, refusals) = laboratory(&stub).await?;
    assert_eq!(measurements(&graph).len(), 1);
    assert_eq!(refused(&refusals, RefusalKind::AmbiguousConcept), 1);
    Ok(())
}

#[tokio::test]
async fn a_code_mapped_to_several_standard_concepts_writes_one_row_per_concept()
-> Result<(), Box<dyn Error>> {
    // CDM conventions, "Mapping": "multiple CONDITION_OCCURRENCE records will be
    // generated for the one source value record".
    let mut stub = StubSource::laboratory()?;
    stub.code(
        "LOINC",
        "SYN-LAB-2",
        mapped(
            concept(2_002, "Measurement", "LOINC", "SYN-LAB-2", false)?,
            vec![
                concept(1_002, "Measurement", "FB-STANDARD", "STD-2", true)?,
                concept(1_003, "Measurement", "FB-STANDARD", "STD-3", true)?,
            ],
        ),
    );
    let (graph, refusals) = laboratory(&stub).await?;
    assert!(refusals.is_empty(), "{refusals:?}");
    let second: Vec<Branch<'_>> = measurements(&graph)
        .into_iter()
        .filter(|row| text(row, "measurement_source_value") == Some("SYN-LAB-2"))
        .map(|row| {
            (
                int(row, "measurement_concept_id"),
                int(row, "measurement_source_concept_id"),
                text(row, "measurement_source_value"),
                row.key().discriminator().branch(),
            )
        })
        .collect();
    assert_eq!(
        second,
        vec![
            (Some(1_002), Some(2_002), Some("SYN-LAB-2"), 0),
            (Some(1_003), Some(2_002), Some("SYN-LAB-2"), 1),
        ]
    );
    // Each branch is a row of its own, so each links to the specimen.
    assert_eq!(graph.links().len(), 3);
    Ok(())
}

#[tokio::test]
async fn a_unit_mapped_to_several_standard_concepts_refuses_the_record()
-> Result<(), Box<dyn Error>> {
    let mut stub = StubSource::laboratory()?;
    stub.code(
        "UCUM",
        "g/L",
        mapped(
            concept(3_002, "Unit", "UCUM", "g/L", false)?,
            vec![
                concept(3_003, "Unit", "UCUM", "g/L-a", true)?,
                concept(3_004, "Unit", "UCUM", "g/L-b", true)?,
            ],
        ),
    );
    let (_, refusals) = laboratory(&stub).await?;
    assert_eq!(refused(&refusals, RefusalKind::SeveralStandardConcepts), 1);
    assert_eq!(refusals.first().and_then(|r| r.column), Some("unit"));
    Ok(())
}

#[tokio::test]
async fn a_record_refusal_names_its_table_and_column_in_the_graph_report()
-> Result<(), Box<dyn Error>> {
    let mut stub = StubSource::laboratory()?;
    stub.code("LOINC", "SYN-LAB-2", Stubbed::Ambiguous(vec![2_006, 2_007]));
    let (graph, _) = laboratory(&stub).await?;
    let refusal = graph.report().refusals().first().expect("a refusal");
    assert_eq!(refusal.table(), Some("measurement"));
    assert_eq!(refusal.column(), Some("concept_id"));
    assert_eq!(
        refusal.mapping().map(omop_cdm::graph::MappingName::as_str),
        Some("Laboratory_test_analyte_v1")
    );
    Ok(())
}

#[tokio::test]
async fn each_distinct_question_is_asked_once() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(
        &index,
        &flat(
            &[(
                &format!("{EVENT}/synthetic_analyte:1/analyte_name|code"),
                json!("SYN-LAB-1"),
            )],
            &[],
        )?,
    )?;
    outcome(&[], &index, &composition, &stub).await?;
    let asked = stub.asked();
    let loinc: Vec<&String> = asked
        .iter()
        .filter(|question| question.contains("SYN-LAB-1"))
        .collect();
    assert_eq!(loinc.len(), 1, "{asked:?}");
    assert!(
        asked
            .iter()
            .any(|question| question.contains("vocabulary_id 'SNOMED'")),
        "SNOMED-CT is read as SNOMED: {asked:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_vocabulary_lookup_refuses_the_run() -> Result<(), Box<dyn Error>> {
    let mut stub = StubSource::laboratory()?;
    stub.fail_operators = true;
    let index = template()?;
    let composition = ranged(&index, "<", "mmol/L")?;
    let program = compiled(&set(LABORATORY, &[])?, &index)?;
    let aliases = VocabularyAliases::default();
    let result = run(
        &program,
        &composition,
        &index,
        &source()?,
        None,
        &seams(&stub, &aliases),
    )
    .await;
    assert!(matches!(result, Err(EngineError::Lookup(_))), "{result:?}");
    Ok(())
}

#[tokio::test]
async fn a_composition_of_another_template_refuses_the_run() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let built = composition(&index, &flat(&[], &[])?)?;
    let other = CanonicalComposition::new(
        built.value().clone(),
        "ferrobridge.other.v1",
        built.generation(),
    );
    let program = compiled(&set(LABORATORY, &[])?, &index)?;
    let aliases = VocabularyAliases::default();
    let result = run(
        &program,
        &other,
        &index,
        &source()?,
        None,
        &seams(&stub, &aliases),
    )
    .await;
    assert!(
        matches!(result, Err(EngineError::TemplateMismatch { .. })),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_element_no_mapping_reads_is_reported_unmapped() -> Result<(), Box<dyn Error>> {
    let stub = StubSource::laboratory()?;
    let index = template()?;
    let composition = composition(&index, &flat(&[], &[])?)?;
    let file = analyte("Synthetic_bare_v1", CONCEPT_AND_DATE);
    let (graph, _) = outcome(
        &[("Synthetic_bare_v1.yml", &file)],
        &index,
        &composition,
        &stub,
    )
    .await?;
    let unmapped: Vec<(&str, &str)> = graph
        .report()
        .unmapped_fields()
        .iter()
        .map(|field| (field.mapping().as_str(), field.element()))
        .collect();
    let first_result = format!("{FIRST_ANALYTE}/items[at0001 and 1]");
    assert!(
        unmapped.contains(&("Synthetic_bare_v1", first_result.as_str())),
        "{unmapped:?}"
    );
    let first_name = format!("{FIRST_ANALYTE}/items[at0024 and 1]");
    assert!(
        !unmapped.iter().any(|(_, element)| *element == first_name),
        "a read element is not unmapped"
    );
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

#[test]
fn the_default_aliases_read_snomed_ct_as_snomed_and_pass_the_rest_through() {
    let aliases = VocabularyAliases::default();
    assert_eq!(aliases.vocabulary_of("SNOMED-CT"), "SNOMED");
    assert_eq!(aliases.vocabulary_of("LOINC"), "LOINC");
    assert_eq!(aliases.vocabulary_of("FB-LOCAL"), "FB-LOCAL");
}
