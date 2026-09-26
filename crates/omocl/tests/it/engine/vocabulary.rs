// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How the engine resolves codes and units against the OHDSI vocabulary.

use std::error::Error;

use omocl::engine::EngineError;
use omocl::engine::RefusalKind;
use omocl::engine::concept::VocabularyAliases;
use omocl::engine::run;
use serde_json::json;

use crate::lab::EVENT;
use crate::lab::LABORATORY;
use crate::lab::StubSource;
use crate::lab::Stubbed;
use crate::lab::compiled;
use crate::lab::composition;
use crate::lab::concept;
use crate::lab::flat;
use crate::lab::mapped;
use crate::lab::seams;
use crate::lab::set;
use crate::lab::source;
use crate::lab::standard;
use crate::lab::template;

use crate::engine::Branch;
use crate::engine::int;
use crate::engine::laboratory;
use crate::engine::measurements;
use crate::engine::outcome;
use crate::engine::ranged;
use crate::engine::refused;
use crate::engine::text;

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
        refusal
            .mapping()
            .map(omop_cdm::graph::key::MappingName::as_str),
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

#[test]
fn the_default_aliases_read_snomed_ct_as_snomed_and_pass_the_rest_through() {
    let aliases = VocabularyAliases::default();
    assert_eq!(aliases.vocabulary_of("SNOMED-CT"), "SNOMED");
    assert_eq!(aliases.vocabulary_of("LOINC"), "LOINC");
    assert_eq!(aliases.vocabulary_of("FB-LOCAL"), "FB-LOCAL");
}
