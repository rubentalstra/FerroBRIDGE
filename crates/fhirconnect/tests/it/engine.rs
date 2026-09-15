// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter over a compiled program.
//!
//! The chain under test is a FerroBRIDGE-authored model mapping carrying one
//! condition per operator of
//! `docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
//! compiled against the synthetic diagnosis template of the testkit and
//! evaluated over the synthetic FHIR `Condition` of the testkit.

use core::error::Error;
use std::sync::Arc;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::condition::Verdict;
use fhirconnect::engine::condition::attached;
use fhirconnect::engine::condition::evaluate;
use fhirconnect::engine::condition::runs;
use fhirconnect::model::ast::ContextMappingFile;
use fhirconnect::model::ast::Direction;
use fhirconnect::model::ast::ModelMappingFile;
use fhirconnect::model::load::MappingSet;
use fhirconnect::model::parse::lower_context;
use fhirconnect::model::parse::lower_model;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Condition;
use fhirconnect::resolve::program::Mapping;
use fhirconnect::resolve::program::Method;
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::loader::load_str;
use openehr_mapping_core::template::TemplateSource;

/// The fixtures this crate's tests ship.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// Builds the index over the testkit's diagnosis template.
fn template() -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::DIAGNOSE_OPT)?;
    Ok(WebTemplateIndex::build(&TemplateSource::Opt14(Box::new(
        opt,
    )))?)
}

/// Compiles the condition fixture into its program.
fn program() -> Result<Arc<Program>, Box<dyn Error>> {
    let mut set = MappingSet::new();
    for (name, text) in [
        (
            "contexts/ferrobridge_conditions.context.yml",
            std::fs::read_to_string(format!(
                "{FIXTURES}/contexts/ferrobridge_conditions.context.yml"
            ))?,
        ),
        (
            "model/ferrobridge_conditions.yml",
            std::fs::read_to_string(format!("{FIXTURES}/model/ferrobridge_conditions.yml"))?,
        ),
    ] {
        let document = load_str(name, &text)?;
        if text.contains("\ntype: context\n") {
            let file: ContextMappingFile =
                lower_context(&document).map_err(|diagnostics| render(&diagnostics))?;
            set.insert_context(file)
                .map_err(|error| error.to_string())?;
        } else {
            let file: ModelMappingFile =
                lower_model(&document).map_err(|diagnostics| render(&diagnostics))?;
            set.insert_model(file).map_err(|error| error.to_string())?;
        }
    }
    let index = template()?;
    let context = MappingName::new("ferrobridge_conditions.context")?;
    compile(
        &set,
        &context,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .map_err(|diagnostics| Box::<dyn Error>::from(render(&diagnostics)))
}

/// Renders a diagnostic list as one error message.
fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {}: {}",
                diagnostic.file().display(),
                diagnostic.code(),
                diagnostic.message()
            )
        })
        .collect::<Vec<String>>()
        .join("; ")
}

/// Returns the compiled mapping of `name`, at any depth.
fn mapping<'program>(program: &'program Program, name: &str) -> Option<&'program Mapping> {
    fn walk<'mapping>(mappings: &'mapping [Mapping], name: &str) -> Option<&'mapping Mapping> {
        for mapping in mappings {
            if mapping.name() == name {
                return Some(mapping);
            }
            if let Some(found) = walk(mapping.followed_by(), name) {
                return Some(found);
            }
            let nested = match *mapping.method() {
                Method::Slot { ref mappings, .. } | Method::Reference { ref mappings, .. } => {
                    mappings.as_slice()
                }
                Method::Value
                | Method::Link { .. }
                | Method::Programmed { .. }
                | Method::Participation { .. } => &[],
            };
            if let Some(found) = walk(nested, name) {
                return Some(found);
            }
        }
        None
    }
    walk(program.mappings(), name)
}

/// Returns the synthetic FHIR `Condition` of the testkit as a lexical tree.
fn condition_document() -> Result<Value, Box<dyn Error>> {
    let parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    Ok(Value::from_serde_json(parsed))
}

/// Evaluates the `fhirCondition` of the mapping named `name`.
fn verdict(name: &str) -> Result<Verdict, Box<dyn Error>> {
    let program = program()?;
    let mapping = mapping(&program, name).ok_or_else(|| format!("no mapping named {name}"))?;
    let condition = mapping
        .fhir_condition()
        .ok_or_else(|| format!("{name} carries no fhirCondition"))?;
    let input = mapping
        .fhir()
        .ok_or_else(|| format!("{name} carries no FHIR path"))?;
    let document = condition_document()?;
    Ok(evaluate(
        &SCHEMAS,
        &document,
        condition,
        attached(condition, input.expression()),
    )?)
}

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
    let filtering_input = filtering.fhir().ok_or("it carries a FHIR path")?;
    let gating_input = gating.fhir().ok_or("it carries a FHIR path")?;
    assert!(
        attached(
            filtering.fhir_condition().ok_or("a fhirCondition")?,
            filtering_input.expression()
        ),
        "a targetRoot equal to the with path filters it"
    );
    assert!(
        !attached(
            gating.fhir_condition().ok_or("a fhirCondition")?,
            gating_input.expression()
        ),
        "a targetRoot pointing elsewhere is a gate"
    );
    Ok(())
}
