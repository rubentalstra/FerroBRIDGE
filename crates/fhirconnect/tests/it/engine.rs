// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter over a compiled program.
//!
//! The chain under test is a FerroBRIDGE-authored model mapping carrying one
//! condition per operator of
//! `docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
//! compiled against the synthetic diagnosis template of the testkit and
//! evaluated over the synthetic FHIR `Condition` of the testkit.

use core::cell::RefCell;
use core::error::Error;
use std::collections::BTreeMap;
use std::sync::Arc;

use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::condition::Verdict;
use fhirconnect::engine::condition::evaluate;
use fhirconnect::engine::condition::runs;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::origin::Origin;
use fhirconnect::engine::origin::SourceItem;
use fhirconnect::engine::outcome::SkipReason;
use fhirconnect::engine::outcome::Warning;
use fhirconnect::engine::seam::IdentityRequest;
use fhirconnect::engine::seam::IdentitySink;
use fhirconnect::engine::seam::ReferenceError;
use fhirconnect::engine::seam::ReferenceSource;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::Defaults;
use fhirconnect::engine::traverse::EngineError;
use fhirconnect::engine::traverse::SplitRefusal;
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
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    insta::assert_json_snapshot!("diagnosis_composition", inbound.value().value());
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
            &Seams::default(),
            &defaults(),
            &CallContext::new(),
        )?;
        let outbound = to_fhir(
            &program,
            &SCHEMAS,
            &index,
            inbound.value(),
            &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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
        &Seams::default(),
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

/// Returns the defaults of a run whose origin is the synthetic sender.
fn audited(source: SourceItem) -> Defaults {
    defaults().with_origin(Origin::new("ferrobridge.test").with_source(source))
}

/// Returns the `id` of every entry of one `FEEDER_AUDIT` identifier list.
fn audit_ids<'value>(audit: &'value serde_json::Value, list: &str) -> Vec<&'value str> {
    audit
        .get(list)
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_run_with_an_origin_records_it_in_the_feeder_audit() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 common.html §FEEDER_AUDIT: the audit "describes the origin of
    // data that have been transformed into openEHR form", and
    // FEEDER_AUDIT_DETAILS.system_id names the system that handled it.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &audited(
            SourceItem::new("Condition")
                .with_id("sender-1")
                .with_version_id("3"),
        ),
        &CallContext::new(),
    )?;
    let audit = inbound
        .value()
        .value()
        .get("feeder_audit")
        .ok_or("the composition carries a FEEDER_AUDIT")?;
    assert_eq!(
        audit
            .pointer("/originating_system_audit/system_id")
            .and_then(serde_json::Value::as_str),
        Some("ferrobridge.test")
    );
    assert_eq!(
        audit
            .pointer("/originating_system_audit/version_id")
            .and_then(serde_json::Value::as_str),
        Some("3"),
        "the source meta.versionId travels as the originating version"
    );
    assert_eq!(
        audit_ids(audit, "originating_system_item_ids"),
        ["sender-1"],
        "the source resource id travels as the one originating item: {audit}"
    );
    assert_eq!(
        audit
            .pointer("/originating_system_item_ids/0/type")
            .and_then(serde_json::Value::as_str),
        Some("Condition")
    );
    assert_eq!(
        audit
            .pointer("/feeder_system_audit/system_id")
            .and_then(serde_json::Value::as_str),
        Some("ferrobridge.test")
    );
    Ok(())
}

#[test]
fn the_defaulted_warnings_and_the_feeder_audit_agree() -> Result<(), Box<dyn Error>> {
    // defaults-for-fields.adoc names the fields the engine fills; each one is
    // a declared Warning::Defaulted and one feeder_system_item_ids entry, in
    // the same order.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &audited(SourceItem::new("Condition").with_id("sender-1")),
        &CallContext::new(),
    )?;
    let warned: Vec<&str> = inbound
        .warnings()
        .iter()
        .filter_map(|warning| match *warning {
            Warning::Defaulted { ref field } => Some(field.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !warned.is_empty(),
        "the minimal chain leaves fields to default"
    );
    let audit = inbound
        .value()
        .value()
        .get("feeder_audit")
        .ok_or("the composition carries a FEEDER_AUDIT")?;
    assert_eq!(
        audit_ids(audit, "feeder_system_item_ids"),
        warned,
        "the audit lists exactly the defaulted fields, in order: {audit}"
    );
    let types: Vec<Option<&str>> = audit
        .get("feeder_system_item_ids")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| entry.get("type").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        types.iter().all(|kind| *kind == Some("defaulted")),
        "every feeder item is typed as a default: {types:?}"
    );
    // The builder validated the composition against its template before it
    // answered; reading it back through the same validation shows the audit
    // is admitted content.
    index.accept(inbound.value().value().clone())?;
    Ok(())
}

#[test]
fn a_source_with_no_id_is_recorded_as_unknown() -> Result<(), Box<dyn Error>> {
    // An absent identifier is recorded, never invented.
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &audited(SourceItem::new("Condition")),
        &CallContext::new(),
    )?;
    let audit = inbound
        .value()
        .value()
        .get("feeder_audit")
        .ok_or("the composition carries a FEEDER_AUDIT")?;
    assert_eq!(audit_ids(audit, "originating_system_item_ids"), ["unknown"]);
    assert_eq!(
        audit.pointer("/originating_system_audit/version_id"),
        None,
        "a source with no version records none"
    );
    Ok(())
}

#[test]
fn a_run_without_an_origin_writes_no_feeder_audit() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(inbound.value().value().get("feeder_audit"), None);
    Ok(())
}

/// The path of the diagnostic certainty value in the synthetic template.
const CERTAINTY: &str =
    "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]/items[at0073]/value";

/// Returns the synthetic `Condition` with a certainty coding the template's
/// local code list admits.
fn condition_with_local_certainty() -> Result<Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = parsed.as_object_mut() {
        object.insert(
            String::from("verificationStatus"),
            serde_json::json!({
                "coding": [{"system": "local", "code": "at0074", "display": "Confirmed"}],
                "text": "http://example.org/ferrobridge/certainty"
            }),
        );
    }
    Ok(Value::from_serde_json(parsed))
}

/// Returns the certainty value a composition holds.
fn certainty_of(
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let node = index.node(&AqlPath::new(CERTAINTY))?;
    Ok(index
        .read(composition, node, &[])?
        .ok_or("the run wrote the certainty")?)
}

#[test]
fn a_tail_below_a_node_is_written_and_read_in_both_directions() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 data_types.html §DV_CODED_TEXT: defining_code is a CODE_PHRASE
    // whose code_string and terminology_id.value are attributes below the
    // node the template constrains; each mapping names one of them.
    let program = compiled("ferrobridge_tail")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_local_certainty()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let written = certainty_of(&index, inbound.value())?;
    assert_eq!(
        written
            .pointer("/defining_code/code_string")
            .and_then(serde_json::Value::as_str),
        Some("at0074"),
        "the code landed in the coded text's code_string: {written}"
    );
    assert_eq!(
        written
            .pointer("/defining_code/terminology_id/value")
            .and_then(serde_json::Value::as_str),
        Some("local")
    );
    assert_eq!(
        written.get("value").and_then(serde_json::Value::as_str),
        Some("Confirmed")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    let coding = outbound
        .value()
        .get("verificationStatus")
        .and_then(|status| status.get("coding"))
        .ok_or("the Condition carries the certainty coding")?
        .to_serde_json(&mut fhir_types::codec::Path::root("Coding"))?;
    assert_eq!(
        coding,
        serde_json::json!([{"system": "local", "code": "at0074", "display": "Confirmed"}]),
        "the three attributes read back into one coding"
    );
    Ok(())
}

#[test]
fn the_tails_leaf_class_selects_the_cell() -> Result<(), Box<dyn Error>> {
    // The Coding lands on DV_CODED_TEXT.defining_code, a CODE_PHRASE, so the
    // CODE_PHRASE row of the data-type chapter runs, not the DV_CODED_TEXT
    // row of the node.
    let program = compiled("ferrobridge_tail_phrase")?;
    let phrase =
        mapping(&program, "certainty.certaintyPhrase").ok_or("the phrase mapping compiled")?;
    assert_eq!(
        phrase
            .openehr()
            .and_then(fhirconnect::resolve::program::OpenehrTarget::leaf_class),
        Some("CODE_PHRASE"),
        "the resolver records the tail's class"
    );
    assert_eq!(
        phrase.openehr().map(|target| target.node().rm_type()),
        Some("DV_CODED_TEXT")
    );
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_local_certainty()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let written = certainty_of(&index, inbound.value())?;
    assert_eq!(
        written
            .pointer("/defining_code/code_string")
            .and_then(serde_json::Value::as_str),
        Some("at0074")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    let coding = outbound
        .value()
        .get("verificationStatus")
        .and_then(|status| status.get("coding"))
        .and_then(|codings| codings.as_array()?.first())
        .ok_or("the Condition carries the certainty coding")?;
    assert_eq!(coding.get("code").and_then(Value::as_str), Some("at0074"));
    assert_eq!(
        coding.get("display").and_then(Value::as_str),
        Some("Confirmed")
    );
    Ok(())
}

#[test]
fn a_tail_no_flat_part_carries_is_refused_in_both_directions() -> Result<(), Box<dyn Error>> {
    // DV_TEXT.hyperlink is an attribute of the reference model, and the
    // Simplified Formats DV_TEXT table writes no part for it, so a write
    // through it would be lost on the way to the wire.
    let program = compiled("ferrobridge_tail_unsupported")?;
    let index = template()?;
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_local_certainty()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("no FLAT part carries the hyperlink");
    assert!(
        matches!(error, EngineError::UnsupportedTail { ref tail, .. } if tail.contains("hyperlink")),
        "the refusal names the tail: {error}"
    );
    let composition = compiled("ferrobridge_tail").and_then(|carrier| {
        Ok(to_openehr(
            &carrier,
            &SCHEMAS,
            &index,
            &condition_with_local_certainty()?,
            &Seams::default(),
            &defaults(),
            &CallContext::new(),
        )?)
    })?;
    let error = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        composition.value(),
        &Seams::default(),
        &CallContext::new(),
    )
    .expect_err("the read side mirrors the table");
    assert!(
        matches!(error, EngineError::UnsupportedTail { .. }),
        "the read refuses the same tail: {error}"
    );
    Ok(())
}

/// A reference source over a map, keyed by the literal reference.
#[derive(Debug, Default)]
struct MapReferences(BTreeMap<String, Value>);

impl ReferenceSource for MapReferences {
    fn fetch(
        &self,
        reference: &str,
        _expected: &ResourceType,
    ) -> Result<Option<Value>, ReferenceError> {
        Ok(self.0.get(reference).cloned())
    }
}

/// An identity sink over a map from resource type to id, recording every
/// request it answers.
#[derive(Debug, Default)]
struct MapIdentities {
    ids: BTreeMap<String, String>,
    seen: RefCell<Vec<IdentityRequest>>,
}

impl IdentitySink for MapIdentities {
    fn identify(&self, request: &IdentityRequest) -> Result<String, ReferenceError> {
        self.seen.borrow_mut().push(request.clone());
        self.ids
            .get(request.resource_type())
            .cloned()
            .ok_or_else(|| ReferenceError::Identity {
                resource_type: String::from(request.resource_type()),
                source: Box::from("the map holds no id for the type"),
            })
    }
}

/// Returns the synthetic `Condition` with the given members set.
fn condition_with(members: serde_json::Value) -> Result<Value, Box<dyn Error>> {
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let (Some(object), serde_json::Value::Object(added)) = (parsed.as_object_mut(), members) {
        object.extend(added);
    }
    Ok(Value::from_serde_json(parsed))
}

/// Returns the synthetic `Observation` the evidence reference points at.
fn evidence_observation() -> Value {
    Value::from_serde_json(evidence_json())
}

/// Returns the synthetic `Observation` the evidence reference points at, as
/// JSON.
fn evidence_json() -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Observation",
        "id": "synthetic-observation-1",
        "status": "final",
        "code": {"text": "Synthetic evidence one"}
    })
}

/// Returns the text of the problem-diagnosis comment a composition holds.
fn comment_of(
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
) -> Result<Option<String>, Box<dyn Error>> {
    let node = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]/items[at0069]/value",
    ))?;
    Ok(index
        .read(composition, node, &[])?
        .and_then(|value| value.get("value")?.as_str().map(String::from)))
}

#[test]
fn a_reference_maps_the_referenced_resource_into_openehr() -> Result<(), Box<dyn Error>> {
    // Reference.adoc: the `reference` mapping initializes the referenced
    // resource and its mappings run over it, with `$fhirRoot` its root.
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        evidence_observation(),
    );
    let references = MapReferences(held);
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(
        comment_of(&index, inbound.value())?.as_deref(),
        Some("Synthetic evidence one"),
        "the referenced Observation's code reached the composition"
    );
    Ok(())
}

#[test]
fn a_contained_reference_resolves_in_the_document_itself() -> Result<(), Box<dyn Error>> {
    // R4 references.html#contained: `#id` names a contained resource.
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut contained = evidence_json();
    if let Some(object) = contained.as_object_mut() {
        object.insert(String::from("id"), serde_json::json!("evidence"));
    }
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "contained": [contained],
            "evidence": [{"detail": [{"reference": "#evidence"}]}]
        }))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(
        comment_of(&index, inbound.value())?.as_deref(),
        Some("Synthetic evidence one")
    );
    Ok(())
}

#[test]
fn a_reference_that_resolves_to_nothing_is_a_declared_skip() -> Result<(), Box<dyn Error>> {
    // references.adoc: "If not possible, the engine proceeds with the mapping".
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/absent"}]}]
        }))?,
        &Seams::default().with_references(&MapReferences::default()),
        &defaults(),
        &CallContext::new(),
    )?;
    assert!(
        inbound.warnings().contains(&Warning::Skipped {
            mapping: String::from("evidence"),
            reason: SkipReason::UnresolvedReference {
                reference: String::from("Observation/absent"),
            },
        }),
        "the unresolved reference is declared: {:?}",
        inbound.warnings()
    );
    assert_eq!(comment_of(&index, inbound.value())?, None);
    Ok(())
}

#[test]
fn a_reference_of_the_wrong_type_refuses() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        Value::from_serde_json(serde_json::json!({"resourceType": "Specimen", "id": "x"})),
    );
    let references = MapReferences(held);
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("a Specimen is no Observation");
    assert!(
        matches!(error, EngineError::Reference { .. }),
        "the refusal is the reference's: {error}"
    );
    Ok(())
}

#[test]
fn a_reference_out_of_openehr_creates_the_resource_the_sink_names() -> Result<(), Box<dyn Error>> {
    // Reference.adoc: "initialize a new resource in FHIR ... and reference it
    // inside the resource we are currently mapping".
    let program = compiled("ferrobridge_reference")?;
    let index = template()?;
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        evidence_observation(),
    );
    let references = MapReferences(held);
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )?;
    let mut ids = BTreeMap::new();
    ids.insert(
        String::from("Observation"),
        String::from("created-observation-1"),
    );
    let identities = MapIdentities {
        ids,
        seen: RefCell::default(),
    };
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default().with_identities(&identities),
        &CallContext::new(),
    )?;
    let detail = outbound
        .value()
        .get("evidence")
        .and_then(|evidence| evidence.as_array()?.first())
        .and_then(|evidence| evidence.get("detail"))
        .and_then(|detail| detail.as_array()?.first())
        .and_then(|detail| detail.get("reference"))
        .and_then(Value::as_str);
    assert_eq!(detail, Some("Observation/created-observation-1"));
    let created = outbound
        .created()
        .first()
        .ok_or("the run created the Observation")?;
    assert_eq!(
        created.get("id").and_then(Value::as_str),
        Some("created-observation-1")
    );
    assert_eq!(
        created
            .get("code")
            .and_then(|code| code.get("text"))
            .and_then(Value::as_str),
        Some("Synthetic evidence one"),
        "the created Observation carries what its mappings read"
    );
    let seen = identities.seen.borrow();
    assert_eq!(
        seen.iter()
            .map(IdentityRequest::resource_type)
            .collect::<Vec<&str>>(),
        ["Observation"],
        "one identity was asked for"
    );
    Ok(())
}

#[test]
fn a_reference_chain_that_reaches_itself_refuses() -> Result<(), Box<dyn Error>> {
    // references.adoc: the engine should "keep track of which ones are
    // already resolved" to prevent circular dependencies.
    let program = compiled("ferrobridge_reference_cycle")?;
    let index = template()?;
    let mut observation = evidence_json();
    if let Some(object) = observation.as_object_mut() {
        object.insert(
            String::from("hasMember"),
            serde_json::json!([{"reference": "Observation/synthetic-observation-1"}]),
        );
    }
    let mut held = BTreeMap::new();
    held.insert(
        String::from("Observation/synthetic-observation-1"),
        Value::from_serde_json(observation),
    );
    let references = MapReferences(held);
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "evidence": [{"detail": [{"reference": "Observation/synthetic-observation-1"}]}]
        }))?,
        &Seams::default().with_references(&references),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("the chain reaches the Observation twice");
    assert!(
        matches!(error, EngineError::ReferenceCycle { ref reference, .. } if reference == "Observation/synthetic-observation-1"),
        "the refusal names the reference: {error}"
    );
    Ok(())
}

/// A synthetic `ehr:` URI of a composition version the link targets.
const LINKED: &str = "ehr://ferrobridge.example/3a2f1c4e-0000-4000-8000-0000000000ab/compositions/d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1";

#[test]
fn a_link_is_written_and_read_back() -> Result<(), Box<dyn Error>> {
    // concept-mappings.adoc §Linked mappings: the fields of the LINK are the
    // ones the `link` block sets; RM 1.1.0 common.html §LINK.
    let program = compiled("ferrobridge_link")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({"encounter": {"reference": LINKED}}))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let entry = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]",
    ))?;
    let written = index
        .read(inbound.value(), entry, &[])?
        .ok_or("the entry was written")?;
    let link = written
        .pointer("/links/0")
        .ok_or("the entry carries a LINK")?;
    assert_eq!(
        link.pointer("/target/value")
            .and_then(serde_json::Value::as_str),
        Some(LINKED)
    );
    assert_eq!(
        link.pointer("/meaning/value")
            .and_then(serde_json::Value::as_str),
        Some("the encounter the diagnosis was made in")
    );
    assert_eq!(
        link.pointer("/type/value")
            .and_then(serde_json::Value::as_str),
        Some("encounter")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        outbound
            .value()
            .get("encounter")
            .and_then(|encounter| encounter.get("reference"))
            .and_then(Value::as_str),
        Some(LINKED),
        "the link's target reads back into the reference"
    );
    Ok(())
}

#[test]
fn a_link_to_what_no_ehr_uri_names_refuses() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 data_types.html §DV_EHR_URI: the target "has the scheme name
    // 'ehr'", and a relative FHIR reference has none.
    let program = compiled("ferrobridge_link")?;
    let index = template()?;
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({
            "encounter": {"reference": "Encounter/synthetic-encounter-1"}
        }))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("a relative reference is no ehr: URI");
    assert!(
        matches!(error, EngineError::LinkTarget { ref target, .. } if target == "Encounter/synthetic-encounter-1"),
        "the refusal names the target: {error}"
    );
    Ok(())
}

#[test]
fn a_participation_is_written_and_read_back() -> Result<(), Box<dyn Error>> {
    // concept-mappings.adoc §Participation mappings: the function is the
    // method's, the participant the Reference; RM 1.1.0 common.html
    // §PARTICIPATION.
    let program = compiled("ferrobridge_participation")?;
    let index = template()?;
    let asserter = serde_json::json!({
        "reference": "Practitioner/synthetic-practitioner-1",
        "display": "Synthetic Practitioner One"
    });
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with(serde_json::json!({"asserter": asserter}))?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let entry = index.node(&AqlPath::new(
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]",
    ))?;
    let written = index
        .read(inbound.value(), entry, &[])?
        .ok_or("the entry was written")?;
    let participation = written
        .pointer("/other_participations/0")
        .ok_or("the entry carries a PARTICIPATION")?;
    assert_eq!(
        participation
            .pointer("/function/value")
            .and_then(serde_json::Value::as_str),
        Some("asserter")
    );
    assert_eq!(
        participation
            .pointer("/performer/name")
            .and_then(serde_json::Value::as_str),
        Some("Synthetic Practitioner One")
    );
    assert_eq!(
        participation
            .pointer("/performer/external_ref/namespace")
            .and_then(serde_json::Value::as_str),
        Some("Practitioner")
    );
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(
        outbound
            .value()
            .get("asserter")
            .map(|found| found.to_serde_json(&mut fhir_types::codec::Path::root("Reference")))
            .transpose()?,
        Some(asserter),
        "the participant reads back as the Reference it came from"
    );
    Ok(())
}

/// Returns the `bodySite.text` values of one Condition, in order.
fn body_sites(condition: &Value) -> Vec<&str> {
    condition
        .get("bodySite")
        .and_then(Value::as_array)
        .map(|sites| {
            sites
                .iter()
                .filter_map(|site| site.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// Returns the body-site names the anatomical-location clusters of a
/// composition carry, in order.
fn cluster_sites(composition: &CanonicalComposition) -> Vec<String> {
    composition
        .value()
        .pointer("/content/0/data/items")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("archetype_node_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("openEHR-EHR-CLUSTER.anatomical_location.v1")
                })
                .filter_map(|cluster| {
                    cluster
                        .pointer("/items/0/value/value")
                        .and_then(serde_json::Value::as_str)
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns a Condition with one `bodySite` per text.
fn condition_with_sites(texts: &[&str]) -> Result<Value, Box<dyn Error>> {
    let sites: Vec<serde_json::Value> = texts
        .iter()
        .map(|text| serde_json::json!({"text": text}))
        .collect();
    condition_with(serde_json::json!({"bodySite": sites}))
}

#[test]
fn a_split_into_openehr_creates_one_cluster_per_body_site() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §Hierarchy and unique values: from FHIR to
    // openEHR, each occurrence of the `with` path creates a new element.
    let program = compiled("ferrobridge_split")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(cluster_sites(inbound.value()), ["Left knee", "Right knee"]);
    let subject_skips = inbound
        .warnings()
        .iter()
        .filter(|warning| {
            **warning
                == Warning::Skipped {
                    mapping: String::from("subject"),
                    reason: SkipReason::Unidirectional,
                }
        })
        .count();
    assert_eq!(
        subject_skips,
        1,
        "a skip that holds for every group is declared once: {:?}",
        inbound.warnings()
    );
    Ok(())
}

#[test]
fn a_split_out_of_openehr_creates_one_resource_per_cluster() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §split: "for each event in openEHR, one
    // resource must be created", and the composition's other fields reach
    // every one of them.
    let program = compiled("ferrobridge_split")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let identities = MapIdentities {
        ids: BTreeMap::from([(String::from("Condition"), String::from("split-condition-2"))]),
        seen: RefCell::default(),
    };
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default().with_identities(&identities),
        &CallContext::new(),
    )?;
    assert_eq!(body_sites(outbound.value()), ["Left knee"]);
    let [ref second] = *outbound.created() else {
        return Err(Box::from("the second cluster created one more Condition"));
    };
    assert_eq!(body_sites(second), ["Right knee"]);
    assert_eq!(
        second.get("id").and_then(Value::as_str),
        Some("split-condition-2")
    );
    for condition in [outbound.value(), second] {
        assert_eq!(
            condition
                .get("code")
                .and_then(|code| code.get("text"))
                .and_then(Value::as_str),
            Some("Synthetic problem one"),
            "the content outside the split reaches every resource"
        );
    }
    let seen = identities.seen.borrow();
    let [ref request] = *seen.as_slice() else {
        return Err(Box::from("one identity was asked for"));
    };
    assert_eq!(
        request.occurrence(),
        [2],
        "the split occurrence travels in the identity request"
    );
    Ok(())
}

#[test]
fn a_split_derives_the_same_ids_on_every_run() -> Result<(), Box<dyn Error>> {
    // The default identity sink derives the id from the request, so the same
    // composition yields the same ids across runs.
    let program = compiled("ferrobridge_split")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee", "Left hip"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let ids = || -> Result<Vec<String>, Box<dyn Error>> {
        let outbound = to_fhir(
            &program,
            &SCHEMAS,
            &index,
            inbound.value(),
            &Seams::default(),
            &CallContext::new(),
        )?;
        Ok(outbound
            .created()
            .iter()
            .filter_map(|created| created.get("id").and_then(Value::as_str).map(String::from))
            .collect())
    };
    let first = ids()?;
    assert_eq!(
        first.len(),
        2,
        "two further clusters created two Conditions"
    );
    assert_ne!(
        first.first(),
        first.get(1),
        "each occurrence takes its own id"
    );
    assert_eq!(first, ids()?, "a second run derives the same ids");
    Ok(())
}

#[test]
fn a_split_groups_occurrences_by_their_unique_tuple() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc: "the `unique:` key defines the condition that
    // triggers the creation of the new element", so occurrences that share
    // the tuple share the element.
    let program = compiled("ferrobridge_split_unique")?;
    let index = template()?;
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee", "Left knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(
        cluster_sites(inbound.value()),
        ["Left knee", "Right knee"],
        "two distinct body-site texts create two clusters"
    );
    let identities = MapIdentities {
        ids: BTreeMap::from([(String::from("Condition"), String::from("unique-condition"))]),
        seen: RefCell::default(),
    };
    let outbound = to_fhir(
        &program,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default().with_identities(&identities),
        &CallContext::new(),
    )?;
    assert_eq!(outbound.created().len(), 1);
    let seen = identities.seen.borrow();
    assert_eq!(
        seen.first().map(IdentityRequest::unique),
        Some([String::from("Right knee")].as_slice()),
        "the unique tuple travels in the identity request"
    );
    Ok(())
}

#[test]
fn a_slotted_files_split_runs_under_the_slot() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc: the hierarchy lives in the preprocessor of the
    // file it belongs to, so a slotted file's split runs over the slotted
    // mappings.
    let split = compiled("ferrobridge_split")?;
    let index = template()?;
    let model = MappingName::new("ferrobridge_split")?;
    let slot = Mapping::new(MappingParts {
        method: Method::Slot {
            model: model.clone(),
            preprocessors: vec![Preprocessor::new(
                model.clone(),
                None,
                None,
                split.hierarchy().cloned(),
            )],
            mappings: split.mappings().to_vec(),
        },
        ..slot_parts(&model, Vec::new())
    });
    let host = cyclic_program(&MappingName::new("ferrobridge_slot_host")?, vec![slot])?;
    let inbound = to_openehr(
        &host,
        &SCHEMAS,
        &index,
        &condition_with_sites(&["Left knee", "Right knee"])?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    assert_eq!(cluster_sites(inbound.value()), ["Left knee", "Right knee"]);
    let outbound = to_fhir(
        &host,
        &SCHEMAS,
        &index,
        inbound.value(),
        &Seams::default(),
        &CallContext::new(),
    )?;
    assert_eq!(body_sites(outbound.value()), ["Left knee"]);
    let [ref second] = *outbound.created() else {
        return Err(Box::from("the slotted split created one more Condition"));
    };
    assert_eq!(body_sites(second), ["Right knee"]);
    Ok(())
}

#[test]
fn a_split_on_a_node_that_does_not_repeat_refuses() -> Result<(), Box<dyn Error>> {
    // HierarchyMappings.adoc §split: an element is created per occurrence, so
    // a node the template holds once takes no second one.
    let split = compiled("ferrobridge_split")?;
    let minimal = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let problem = mapping(&minimal, "problemDiagnose").ok_or("the problem mapping")?;
    let fhir = problem.fhir().cloned();
    let openehr = problem.openehr().cloned();
    let hierarchy = fhirconnect::resolve::program::Hierarchy::new(
        fhir,
        openehr,
        None,
        split
            .hierarchy()
            .and_then(|found| found.split_openehr())
            .cloned(),
    );
    let model = MappingName::new("ferrobridge_split")?;
    let slot = Mapping::new(MappingParts {
        method: Method::Slot {
            model: model.clone(),
            preprocessors: vec![Preprocessor::new(
                model.clone(),
                None,
                None,
                Some(hierarchy),
            )],
            mappings: split.mappings().to_vec(),
        },
        ..slot_parts(&model, Vec::new())
    });
    let host = cyclic_program(&MappingName::new("ferrobridge_slot_host")?, vec![slot])?;
    let error = to_openehr(
        &host,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("the problem name occurs once");
    assert!(
        matches!(
            error,
            EngineError::Split {
                reason: SplitRefusal::NotRepeating { .. },
                ..
            }
        ),
        "the refusal names the node: {error}"
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

#[test]
fn a_manual_path_no_flat_part_carries_is_refused() -> Result<(), Box<dyn Error>> {
    // DV_TEXT.hyperlink is an RM attribute the Simplified Formats DV_TEXT table
    // writes no part for, so a manual write through it would be lost.
    let program = compiled("ferrobridge_manual_unsupported")?;
    let index = template()?;
    let error = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &condition_document()?,
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )
    .expect_err("no FLAT part carries the hyperlink");
    assert!(
        matches!(
            error,
            EngineError::UnsupportedTail { ref mapping, ref tail, .. }
                if mapping == "certainty.linked" && tail.contains("hyperlink")
        ),
        "the refusal names the manual entry and the tail: {error}"
    );
    Ok(())
}
