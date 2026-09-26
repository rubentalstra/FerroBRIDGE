// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The conditions, each evaluated on the input side.

use core::error::Error;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::condition::Verdict;
use fhirconnect::engine::condition::runs;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::error::EngineError;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::model::ast::keyword::Direction;

use fhirconnect::resolve::program::condition::Condition;
use fhirconnect::resolve::program::target::Attachment;

use crate::support::compiled;
use crate::support::template;

use crate::engine::chain::onset_composition;
use crate::engine::mapping;
use crate::engine::program;
use crate::engine::verdict;

#[test]
fn a_fhir_condition_runs_only_when_fhir_is_the_input() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc: "When we map a FHIR resource into openEHR, only the
    // fhirConditions are processed and in case of openEHR to FHIR it's the
    // other way around."
    let program = program()?;
    let mapping = mapping(&program, "identifierOneOf").ok_or("the identifier mapping compiled")?;
    let condition: &Condition = mapping
        .fhir_condition()
        .ok_or("it carries a fhirCondition")?;
    assert!(
        runs(condition, Direction::FhirToOpenehr),
        "a fhirCondition runs when FHIR is the input"
    );
    assert!(
        !runs(condition, Direction::OpenehrToFhir),
        "a fhirCondition never runs when openEHR is the input"
    );
    Ok(())
}

#[test]
fn an_openehr_condition_never_runs_when_fhir_is_the_input() -> Result<(), Box<dyn Error>> {
    let program = program()?;
    let mapping = mapping(&program, "onsetWhenCertaintyPresent").ok_or("the onset mapping")?;
    let condition = mapping
        .openehr_condition()
        .ok_or("it carries an openehrCondition")?;
    assert!(
        !runs(condition, Direction::FhirToOpenehr),
        "an openehrCondition never filters a FHIR input"
    );
    assert!(
        runs(condition, Direction::OpenehrToFhir),
        "an openehrCondition runs when openEHR is the input"
    );
    assert!(
        mapping.fhir_condition().is_none(),
        "the mapping carries no condition for the other direction"
    );
    Ok(())
}

#[test]
fn a_unidirectional_mapping_names_the_one_direction_it_runs_in() -> Result<(), Box<dyn Error>> {
    let program = program()?;
    let mapping =
        mapping(&program, "recordedDateOutboundOnly").ok_or("the recorded-date mapping")?;
    assert_eq!(
        mapping.direction(),
        Some(Direction::OpenehrToFhir),
        "the program carries the one direction the mapping runs in"
    );
    Ok(())
}

#[test]
fn one_of_admits_the_occurrence_whose_attribute_matches() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §one of: the criteria combine with OR, and an attached
    // condition returns "the identifier entries that match".
    let admitted = verdict("identifierOneOf")?;
    assert_eq!(
        admitted,
        Verdict::Filter(vec![fhirconnect::tree::Occurrence::new([0])]),
        "the one identifier of the fixture carries the criteria system"
    );
    Ok(())
}

#[test]
fn not_of_admits_the_occurrence_the_criteria_leave_alone() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §not of: "checks if the path does not contain one of the
    // elements contained in criteria", combining with AND.
    let admitted = verdict("identifierNotOf")?;
    assert!(
        admitted.admits_any(),
        "the identifier carries no retired system, so it survives the filter"
    );
    Ok(())
}

#[test]
fn empty_passes_when_the_path_holds_nothing() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §empty: the synthetic Condition carries no bodySite, so
    // an unattached `empty` on it is a plain true.
    assert_eq!(
        verdict("bodySiteEmpty")?,
        Verdict::Gate(true),
        "an absent path is empty"
    );
    Ok(())
}

#[test]
fn not_empty_passes_when_the_path_holds_something() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        verdict("codeNotEmpty")?,
        Verdict::Gate(true),
        "the synthetic Condition carries one code coding"
    );
    Ok(())
}

#[test]
fn type_matches_the_element_the_table_resolved() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §type: "checks if the element in the path matches the
    // given type in criteria"; Condition.code is a CodeableConcept
    // (<https://hl7.org/fhir/R4/condition.html>).
    assert_eq!(
        verdict("codeType")?,
        Verdict::Gate(true),
        "Condition.code resolves to CodeableConcept"
    );
    Ok(())
}

#[test]
fn an_attached_condition_filters_and_an_unattached_one_gates() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §targetRoot: a targetRoot that is the `with` path filters
    // its occurrences; one pointing elsewhere is "handled as simple
    // true/false".
    let program = program()?;
    let filtering = mapping(&program, "identifierOneOf").ok_or("the identifier mapping")?;
    let gating = mapping(&program, "bodySiteEmpty").ok_or("the body-site mapping")?;
    assert_eq!(
        filtering
            .fhir_condition()
            .ok_or("a fhirCondition")?
            .attachment(),
        Attachment::Element,
        "a targetRoot equal to the with path filters it"
    );
    assert_eq!(
        gating
            .fhir_condition()
            .ok_or("a fhirCondition")?
            .attachment(),
        Attachment::Unrelated,
        "a targetRoot pointing elsewhere is a gate"
    );
    Ok(())
}

#[test]
fn an_openehr_condition_filters_when_openehr_is_the_input() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc: "in case of openEHR to FHIR it's the other way around",
    // so the openehrCondition decides whether the mapping runs at all.
    let program = compiled("ferrobridge_openehr_condition")?;
    let index = template()?;
    let without = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        &onset_composition(&index, None)?,
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        without.value().get("onsetDateTime"),
        None,
        "the condition holds nothing, so the mapping does not run"
    );
    let with = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        &onset_composition(&index, Some("the comment"))?,
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        with.value().get("onsetDateTime").and_then(Value::as_str),
        Some("2026-09-12T09:00:00+02:00"),
        "the condition holds, so the mapping runs"
    );
    Ok(())
}

#[test]
fn the_contexts_openehr_condition_gates_a_run_out_of_openehr() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §Conditions in the preprocessor: the condition "defines
    // that the mapping file is only executed if the given condition is met",
    // read on the input side, which is openEHR here.
    let program = compiled("ferrobridge_context_gate")?;
    let index = template()?;
    let error = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        &onset_composition(&index, None)?,
        &Seams::default(),
        &CallContext::new(),
    )
    .expect_err("a composition with no comment is not admitted");
    assert!(
        matches!(error, EngineError::NotApplicable { .. }),
        "the refusal is the context's: {error}"
    );
    let admitted = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        &onset_composition(&index, Some("the comment"))?,
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        admitted
            .value()
            .get("onsetDateTime")
            .and_then(Value::as_str),
        Some("2026-09-12T09:00:00+02:00"),
        "a composition the condition admits maps"
    );
    Ok(())
}
