// SPDX-FileCopyrightText: Vernum Projecten B.V.
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
use fhirconnect::engine::condition::evaluate;
use fhirconnect::engine::condition::runs;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::outcome::SkipReason;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::engine::traverse::Defaults;
use fhirconnect::engine::traverse::NoMappingFunctions;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;
use fhirconnect::model::ast::Direction;
use fhirconnect::resolve::program::Attachment;
use fhirconnect::resolve::program::Condition;
use fhirconnect::resolve::program::Mapping;
use fhirconnect::resolve::program::MappingParts;
use fhirconnect::resolve::program::Method;
use fhirconnect::resolve::program::Pin;
use fhirconnect::resolve::program::Preprocessor;
use fhirconnect::resolve::program::ProfileBinding;
use fhirconnect::resolve::program::ProfileUrl;
use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::ProgramParts;
use fhirconnect::resolve::program::ResourceType;
use fhirconnect::resolve::program::TemplateBinding;
use fhirconnect::resolve::program::TemplateId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::index::AqlPath;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::Generation;

use crate::support::compiled;
use crate::support::template;

/// Compiles the condition fixture into its program.
fn program() -> Result<Arc<Program>, Box<dyn Error>> {
    compiled("ferrobridge_conditions")
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
    let document = condition_document()?;
    Ok(evaluate(
        &SCHEMAS,
        &document,
        condition,
        condition.attachment() == Attachment::Element,
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

/// The timestamp the defaults fill the context start time from.
const NOW: &str = "2026-09-13T08:00:00Z";

/// Returns the defaults a run into openEHR fills its composition fields with.
fn defaults() -> Defaults {
    Defaults::at(NOW).with_language("en").with_territory("NL")
}

/// Returns the synthetic FHIR `Condition` with `onset` set to `onset`.
fn condition_with_onset(onset: &str) -> Result<Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("onsetDateTime"),
            serde_json::Value::String(String::from(onset)),
        );
    }
    Ok(Value::from_serde_json(parsed))
}

#[test]
fn the_minimal_diagnosis_chain_maps_both_ways() -> Result<(), Box<dyn Error>> {
    // The chain is EVALUATION.problem_diagnosis.v1's problem name and date of
    // onset against Condition.code and Condition.onset[x]
    // (<https://hl7.org/fhir/R4/condition.html>).
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let document = condition_with_onset("2026-09-12T09:00:00+02:00")?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &document,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )?;
    insta::assert_json_snapshot!("diagnosis_composition", inbound.value().value());
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &NoMappingFunctions,
        &CallContext::new(),
    )?;
    insta::assert_json_snapshot!(
        "diagnosis_condition",
        outbound
            .value()
            .to_serde_json(&mut fhir_types::codec::Path::root("Condition"))?
    );
    Ok(())
}

#[test]
fn a_unidirectional_mapping_is_skipped_and_recorded() -> Result<(), Box<dyn Error>> {
    // The `unidirectional` key names the one direction a mapping runs in, and
    // the skip is a declared outcome rather than a log line.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let document = condition_document()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &document,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )?;
    assert!(
        inbound.warnings().contains(&Warning::Skipped {
            mapping: String::from("recordedDateOutboundOnly"),
            reason: SkipReason::Unidirectional,
        }),
        "the skipped set names the mapping: {:?}",
        inbound.warnings()
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &NoMappingFunctions,
        &CallContext::new(),
    )?;
    assert!(
        !outbound.warnings().iter().any(|warning| matches!(
            *warning,
            Warning::Skipped {
                reason: SkipReason::Unidirectional,
                ..
            }
        )),
        "the same mapping runs in its own direction: {:?}",
        outbound.warnings()
    );
    Ok(())
}

#[test]
fn a_date_and_time_keeps_its_text_in_both_directions() -> Result<(), Box<dyn Error>> {
    // A FHIR dateTime is a lexical form with its own offset
    // (<https://hl7.org/fhir/R4/datatypes.html#dateTime>), and so is the
    // openEHR DV_DATE_TIME, so the text moves byte for byte.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    for written in [
        "2026-09-13T10:00:00Z",
        "2026-09-13T10:00:00+00:00",
        "2026-09-13T10:00:00+01:00",
        "2026-09-13T10:00:00.123456+02:00",
    ] {
        let document = condition_with_onset(written)?;
        let inbound = to_openehr(
            &program,
            &SCHEMAS,
            &index,
            &document,
            &NoMappingFunctions,
            &defaults(),
            &CallContext::new(),
        )?;
        let outbound = to_fhir(
            &program,
            &SCHEMAS,
            &index,
            inbound.value(),
            &NoMappingFunctions,
            &CallContext::new(),
        )?;
        assert_eq!(
            outbound
                .value()
                .get("onsetDateTime")
                .and_then(Value::as_str),
            Some(written),
            "the offset and the fractional seconds survive both ways"
        );
    }
    Ok(())
}

#[test]
fn a_value_the_element_does_not_admit_names_the_element() -> Result<(), Box<dyn Error>> {
    // Coercion is strict: a value that does not read as the element the
    // mapping names refuses the unit rather than being carried as text.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("code"),
            serde_json::Value::String(String::from("a problem, as text")),
        );
    }
    let document = Value::from_serde_json(parsed);
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &document,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("a string is no CodeableConcept");
    assert!(
        error.to_string().contains("Condition.code"),
        "the refusal names the element: {error}"
    );
    Ok(())
}

#[test]
fn a_required_child_the_input_does_not_carry_refuses_the_unit() -> Result<(), Box<dyn Error>> {
    // Fail.adoc: "if we have a parent node with a 1..1 cardinality and a child
    // node with a 1..1 cardinality, the mapping should fail if the child is
    // not provided". at0002 is 1..1 in the synthetic template.
    let program = compiled("ferrobridge_diagnose_required")?;
    let index = template()?;
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.remove("code");
    }
    let document = Value::from_serde_json(parsed);
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &document,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("the required child is not provided");
    assert!(
        error.to_string().contains("items[at0002]"),
        "the refusal names the node the template requires: {error}"
    );
    Ok(())
}

#[test]
fn the_same_chain_maps_when_the_required_child_is_carried() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_required")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )?;
    assert!(
        inbound
            .value()
            .value()
            .to_string()
            .contains("Synthetic problem one"),
        "the child wrote the problem name"
    );
    Ok(())
}

#[test]
fn a_slot_chain_that_reaches_a_model_twice_refuses() -> Result<(), Box<dyn Error>> {
    // SlotArchetypes.adoc lets a model mapping call another by name, so a
    // chain can close on itself; the compiler refuses one and the engine
    // asserts it rather than recursing.
    let index = template()?;
    let model = MappingName::new("ferrobridge_diagnose_minimal")?;
    let inner = Mapping::new(slot_parts(&model, Vec::new()));
    let outer = Mapping::new(slot_parts(&model, vec![inner]));
    let program = cyclic_program(&model, vec![outer])?;
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("the chain reaches the same model twice");
    assert!(
        error.to_string().contains("ferrobridge_diagnose_minimal"),
        "the refusal names the model the chain reaches again: {error}"
    );
    Ok(())
}

#[test]
fn a_slotted_files_preprocessor_gate_skips_the_slot_when_it_closes() -> Result<(), Box<dyn Error>> {
    // Conditions.adoc §Conditions in the preprocessor: the file-level condition
    // "defines that the mapping file is only executed if the given condition
    // is met". A slotted file keeps its gate, and a closed gate is a recorded
    // skip rather than silence. The problem name is written outside the slot
    // so the composition builds either way; the onset lives in the slotted
    // file behind a gate that holds while the Condition has no coded bodySite.
    let index = template()?;
    let conditions = program()?;
    let gate = mapping(&conditions, "bodySiteEmpty")
        .and_then(Mapping::fhir_condition)
        .ok_or("the body-site gate compiled")?
        .clone();
    let minimal = compiled("ferrobridge_diagnose_minimal")?;
    let problem = mapping(&minimal, "problemDiagnose").ok_or("the problem mapping")?;
    let onset = mapping(&minimal, "onset").ok_or("the onset mapping")?;
    let model = MappingName::new("ferrobridge_diagnose_minimal")?;
    let slot = Mapping::new(MappingParts {
        method: Method::Slot {
            model: model.clone(),
            preprocessors: vec![Preprocessor::new(model.clone(), Some(gate), None, None)],
            mappings: vec![onset.clone()],
        },
        ..slot_parts(&model, Vec::new())
    });
    let program = cyclic_program(&model, vec![problem.clone(), slot])?;
    let written = "2026-01-02T03:04:05Z";
    let closed_gate = |warning: &Warning| {
        matches!(
            *warning,
            Warning::Skipped {
                reason: SkipReason::PreprocessorGate { .. },
                ..
            }
        )
    };

    let open = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_onset(written)?,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )?;
    assert!(
        open.value().value().to_string().contains(written),
        "a Condition with no bodySite keeps the gate open, so the slotted onset is written"
    );
    assert!(
        !open.warnings().iter().any(closed_gate),
        "an open gate records no skip: {:?}",
        open.warnings()
    );

    let closed = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_onset_and_body_site(written)?,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )?;
    assert!(
        !closed.value().value().to_string().contains(written),
        "a coded bodySite closes the gate, so the slotted onset is not written"
    );
    assert!(
        closed.warnings().contains(&Warning::Skipped {
            mapping: String::from("slot"),
            reason: SkipReason::PreprocessorGate {
                model: String::from("ferrobridge_diagnose_minimal"),
            },
        }),
        "the closed gate is a recorded skip: {:?}",
        closed.warnings()
    );
    Ok(())
}

/// Returns the synthetic FHIR `Condition` with `onset` set and one coded
/// `bodySite` added.
fn condition_with_onset_and_body_site(onset: &str) -> Result<Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("onsetDateTime"),
            serde_json::Value::String(String::from(onset)),
        );
        object.insert(
            String::from("bodySite"),
            serde_json::json!([{
                "coding": [{
                    "system": "http://example.org/fhir/sid/ferrobridge-site",
                    "code": "SYN-SITE-1"
                }]
            }]),
        );
    }
    Ok(Value::from_serde_json(parsed))
}

#[test]
fn two_mappings_into_one_list_append_and_into_one_value_overwrite() -> Result<(), Box<dyn Error>> {
    // TransformingList.adoc: "the FHIR bodySite is 0..n, therefore both
    // entries would be appended into the bodySite". Overwriting.adoc: a
    // later mapping to the same 0..1 path "will overwrite the first one".
    let program = compiled("ferrobridge_recurrence")?;
    let index = template()?;
    let composition = three_texts(&index)?;
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        &composition,
        &NoMappingFunctions,
        &CallContext::new(),
    )?;
    let sites = outbound
        .value()
        .get("bodySite")
        .and_then(Value::as_array)
        .ok_or("the resource carries a bodySite list")?;
    let texts: Vec<Option<&str>> = sites
        .iter()
        .map(|site| site.get("text").and_then(Value::as_str))
        .collect();
    assert_eq!(
        texts,
        [Some("the problem name"), Some("the severity")],
        "both mappings appended into the repeating element"
    );
    assert_eq!(
        outbound
            .value()
            .get("code")
            .and_then(|code| code.get("text"))
            .and_then(Value::as_str),
        Some("the comment"),
        "the later mapping overwrote the element that holds one value"
    );
    Ok(())
}

#[test]
fn a_manual_block_merges_its_paths_into_one_element() -> Result<(), Box<dyn Error>> {
    // manual.adoc: "`defining_code/terminology_id/value` and
    // `defining_code/code_string` are merged together in the same data element
    // and do not overwrite".
    let program = compiled("ferrobridge_manual")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &NoMappingFunctions,
        &defaults(),
        &CallContext::new(),
    )?;
    let node = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]/items[at0073]/value",
    ))?;
    let written = index
        .read(inbound.value(), node, &[])?
        .ok_or("the manual block wrote the element")?;
    assert_eq!(
        written.get("_type").and_then(serde_json::Value::as_str),
        Some("DV_CODED_TEXT"),
        "the three paths built one coded text: {written}"
    );
    assert_eq!(
        written
            .get("defining_code")
            .and_then(|code| code.get("code_string"))
            .and_then(serde_json::Value::as_str),
        Some("at0074")
    );
    assert_eq!(
        written
            .get("defining_code")
            .and_then(|code| code.get("terminology_id"))
            .and_then(|id| id.get("value"))
            .and_then(serde_json::Value::as_str),
        Some("local"),
        "the terminology path did not overwrite the code path"
    );
    assert_eq!(
        written.get("value").and_then(serde_json::Value::as_str),
        Some("Confirmed")
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
        &NoMappingFunctions,
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
        &NoMappingFunctions,
        &CallContext::new(),
    )?;
    assert_eq!(
        with.value().get("onsetDateTime").and_then(Value::as_str),
        Some("2026-09-12T09:00:00+02:00"),
        "the condition holds, so the mapping runs"
    );
    Ok(())
}

/// Builds a composition with the date of onset and an optional comment.
fn onset_composition(
    index: &WebTemplateIndex,
    comment: Option<&str>,
) -> Result<CanonicalComposition, Box<dyn Error>> {
    let root = "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]";
    let mut values = vec![
        NodeValue::new(
            index.node(&AqlPath::new("/composer"))?,
            "FerroBRIDGE".into(),
        )
        .with_datum("name"),
        NodeValue::new(index.node(&AqlPath::new("/language"))?, "en".into()).with_datum("code"),
        NodeValue::new(index.node(&AqlPath::new("/territory"))?, "NL".into()).with_datum("code"),
        NodeValue::new(
            index.node(&AqlPath::new(format!("{root}/items[at0002]/value")))?,
            "the problem name".into(),
        )
        .with_datum("value"),
        NodeValue::new(
            index.node(&AqlPath::new(format!("{root}/items[at0077]/value")))?,
            "2026-09-12T09:00:00+02:00".into(),
        ),
    ];
    if let Some(comment) = comment {
        values.push(
            NodeValue::new(
                index.node(&AqlPath::new(format!("{root}/items[at0069]/value")))?,
                comment.into(),
            )
            .with_datum("value"),
        );
    }
    Ok(index.build_composition(&values, NOW)?)
}

/// Builds a composition carrying three `DV_TEXT` values of the chain.
fn three_texts(index: &WebTemplateIndex) -> Result<CanonicalComposition, Box<dyn Error>> {
    let root = "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]";
    let mut values = vec![
        NodeValue::new(
            index.node(&AqlPath::new("/composer"))?,
            "FerroBRIDGE".into(),
        )
        .with_datum("name"),
        NodeValue::new(index.node(&AqlPath::new("/language"))?, "en".into()).with_datum("code"),
        NodeValue::new(index.node(&AqlPath::new("/territory"))?, "NL".into()).with_datum("code"),
    ];
    for (node, text) in [
        ("items[at0002]", "the problem name"),
        ("items[at0005]", "the severity"),
        ("items[at0069]", "the comment"),
    ] {
        let path = AqlPath::new(format!("{root}/{node}/value"));
        values.push(NodeValue::new(index.node(&path)?, text.into()).with_datum("value"));
    }
    Ok(index.build_composition(&values, NOW)?)
}

/// Returns the parts of a mapping that hands over to `model`.
fn slot_parts(model: &MappingName, mappings: Vec<Mapping>) -> MappingParts {
    MappingParts {
        name: String::from("slot"),
        model: model.clone(),
        fhir: None,
        openehr: None,
        data_type: None,
        value: None,
        direction: None,
        fhir_condition: None,
        openehr_condition: None,
        manual: Vec::new(),
        conceptmap: None,
        method: Method::Slot {
            model: model.clone(),
            preprocessors: Vec::new(),
            mappings,
        },
        followed_by: Vec::new(),
    }
}

/// Returns a program whose only mapping is the slot chain `mappings`.
fn cyclic_program(
    model: &MappingName,
    mappings: Vec<Mapping>,
) -> Result<Arc<Program>, Box<dyn Error>> {
    Ok(Arc::new(Program::new(ProgramParts {
        context: MappingName::new("ferrobridge_cycle.context")?,
        profile: ProfileBinding::new(
            ProfileUrl::new("http://example.org/fhir/StructureDefinition/ferrobridge-cycle"),
            Pin::new(None),
        ),
        template: TemplateBinding::new(
            TemplateId::new(ferrobridge_testkit::fixtures::DIAGNOSE_TEMPLATE_ID),
            Pin::new(None),
            Generation::Adl14,
        ),
        resource: ResourceType::new("Condition"),
        start: model.clone(),
        models: Vec::new(),
        operational: Vec::new(),
        preprocessors: Vec::new(),
        mappings,
    })))
}
