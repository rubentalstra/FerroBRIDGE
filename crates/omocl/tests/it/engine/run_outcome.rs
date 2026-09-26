// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of a whole run and the report of what no mapping reads.

use std::error::Error;

use omocl::engine::EngineError;
use omocl::engine::concept::VocabularyAliases;
use omocl::engine::run;
use openehr_mapping_core::composition::CanonicalComposition;

use crate::lab::LABORATORY;
use crate::lab::StubSource;
use crate::lab::compiled;
use crate::lab::composition;
use crate::lab::flat;
use crate::lab::seams;
use crate::lab::set;
use crate::lab::source;
use crate::lab::template;

use crate::engine::CONCEPT_AND_DATE;
use crate::engine::FIRST_ANALYTE;
use crate::engine::analyte;
use crate::engine::outcome;

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
