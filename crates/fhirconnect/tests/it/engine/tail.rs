// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reference-model tails below a template node.

use core::error::Error;
use std::sync::Arc;

use core::str::FromStr;
use fhir_types::codec::Value;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::context::CallContext;
use fhirconnect::engine::seam::Seams;
use fhirconnect::engine::traverse::error::EngineError;
use fhirconnect::engine::traverse::to_fhir;
use fhirconnect::engine::traverse::to_openehr;

use fhirconnect::resolve::program::Program;
use fhirconnect::resolve::program::manual::Manual;
use fhirconnect::resolve::program::manual::ManualPath;
use fhirconnect::resolve::program::manual::ManualValue;
use fhirconnect::resolve::program::mapping::Derived;
use fhirconnect::resolve::program::mapping::Mapping;
use fhirconnect::resolve::program::mapping::MappingParts;
use fhirconnect::resolve::program::mapping::Method;
use fhirconnect::resolve::program::target::FhirTarget;
use fhirconnect::resolve::program::target::OpenehrTarget;
use fhirconnect::resolve::program::target::Target;
use fhirconnect::tree::path::FhirPath;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::paths::AqlPath;
use openehr_rm::v1_2::paths::RmPath;

use crate::support::compiled;
use crate::support::template;

use crate::engine::chain::cyclic_program;
use crate::engine::condition_document;
use crate::engine::defaults;
use crate::engine::mapping;
use crate::engine::slot_parts;

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

/// Returns the value the node at `path` holds in `composition`.
fn value_at(
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    path: &str,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let node = index.node(&AqlPath::new(path))?;
    Ok(index
        .read(composition, node, &[])?
        .ok_or_else(|| format!("the run wrote nothing at {path}"))?)
}

#[test]
fn every_attribute_the_model_gives_a_value_survives_into_openehr() -> Result<(), Box<dyn Error>> {
    // RM 1.1.0 data_types.html §DV_TEXT gives the text a `language` and a
    // `hyperlink`, and §DV_ORDERED gives a date and time a `normal_range`;
    // Simplified Formats master04 §Raw canonical JSON carries each value
    // whole, so none of the three is lost on the way to the composition.
    let program = compiled("ferrobridge_tail_carried")?;
    let index = template()?;
    let mut parsed: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    let object = parsed.as_object_mut().ok_or("the Condition is an object")?;
    object.insert(
        String::from("verificationStatus"),
        serde_json::json!({
            "coding": [{"system": "local", "code": "at0074", "display": "Confirmed"}],
            "text": "http://example.org/ferrobridge/certainty"
        }),
    );
    object.insert(
        String::from("severity"),
        serde_json::json!({"coding": [{"system": "ISO_639-1", "code": "de"}]}),
    );
    object.insert(
        String::from("onsetPeriod"),
        serde_json::json!({
            "start": "2026-09-01T08:00:00+02:00",
            "end": "2026-09-02T08:00:00+02:00"
        }),
    );
    let inbound = to_openehr(
        &program,
        &SCHEMAS,
        &index,
        &Value::from_serde_json(parsed),
        &Seams::default(),
        &defaults(),
        &CallContext::new(),
    )?;
    let problem = value_at(
        &index,
        inbound.value(),
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]/items[at0002]/value",
    )?;
    assert_eq!(
        problem.pointer("/language/code_string"),
        Some(&serde_json::json!("de")),
        "DV_TEXT.language survives: {problem}"
    );
    assert_eq!(
        problem.pointer("/language/terminology_id/value"),
        Some(&serde_json::json!("ISO_639-1"))
    );
    let certainty = certainty_of(&index, inbound.value())?;
    assert_eq!(
        certainty.pointer("/hyperlink/value"),
        Some(&serde_json::json!(
            "http://example.org/ferrobridge/certainty"
        )),
        "DV_TEXT.hyperlink survives: {certainty}"
    );
    let onset = value_at(
        &index,
        inbound.value(),
        "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]/items[at0077]/value",
    )?;
    assert_eq!(
        onset.pointer("/value"),
        Some(&serde_json::json!("2026-09-12T10:00:00+02:00"))
    );
    assert_eq!(
        onset.pointer("/normal_range/lower/value"),
        Some(&serde_json::json!("2026-09-01T08:00:00+02:00")),
        "DV_ORDERED.normal_range survives: {onset}"
    );
    assert_eq!(
        onset.pointer("/normal_range/upper/value"),
        Some(&serde_json::json!("2026-09-02T08:00:00+02:00"))
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
        phrase.openehr().and_then(OpenehrTarget::leaf_class),
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

/// Returns a copy of `target` naming `tail` below its node.
///
/// The compiler refuses a tail no FLAT part carries at load
/// (`fc-uncarried-tail`), so the engine's own refusal is reached only through
/// a program the compiler never saw, which this builds.
fn with_tail(target: &OpenehrTarget, tail: &str) -> Result<OpenehrTarget, Box<dyn Error>> {
    let path: RmPath = format!("{}/value/{tail}", target.path()).parse()?;
    Ok(OpenehrTarget::new(
        path,
        target.node().clone(),
        tail.parse()?,
        target.occurrences().to_vec(),
    ))
}

/// Returns the parts of a plain value mapping of the tail carrier.
fn value_parts(
    name: &str,
    fhir: FhirTarget,
    openehr: OpenehrTarget,
) -> Result<MappingParts, Box<dyn Error>> {
    Ok(MappingParts {
        method: Method::Value,
        name: String::from(name),
        fhir: Some(fhir),
        openehr: Some(openehr),
        ..slot_parts(&MappingName::new("ferrobridge_tail")?, Vec::new())
    })
}

/// Resolves `expression` against the R4 `Condition`.
fn condition_target(expression: &str) -> Result<FhirTarget, Box<dyn Error>> {
    let path = FhirPath::from_str(expression)?;
    let resolved = fhirconnect::tree::element::resolve(&SCHEMAS, "Condition", &path)?;
    Ok(FhirTarget::new(path, resolved))
}

/// Returns the tail carrier's program with `certainty` in place of its own
/// certainty mapping, as a program the compiler never saw.
fn uncompiled(certainty: Mapping) -> Result<Arc<Program>, Box<dyn Error>> {
    let carrier = compiled("ferrobridge_tail")?;
    let problem = mapping(&carrier, "problemName").ok_or("the problem mapping")?;
    cyclic_program(
        &MappingName::new("ferrobridge_tail")?,
        vec![problem.clone(), certainty],
    )
}

#[test]
fn a_tail_no_flat_part_carries_is_refused_in_both_directions() -> Result<(), Box<dyn Error>> {
    // DV_TEXT.mappings is a list of the reference model, and the engine walks
    // a tail by attribute name into one value, so a write through it would be
    // lost on the way to the wire. The compiler refuses it at load; this is
    // the engine's backstop.
    let carrier = compiled("ferrobridge_tail")?;
    let node = mapping(&carrier, "certainty")
        .and_then(Mapping::openehr)
        .ok_or("the certainty node")?;
    let program = uncompiled(Mapping::new(MappingParts {
        derived: Some(Derived::Element("string")),
        ..value_parts(
            "certaintyLink",
            condition_target("$resource.verificationStatus.text")?,
            with_tail(node, "mappings/match")?.with_leaf_class("String"),
        )?
    }))?;
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
    .expect_err("the engine carries no tail through a list");
    assert!(
        matches!(error, EngineError::UnsupportedTail { ref tail, .. } if tail.contains("mappings")),
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

#[test]
fn a_manual_path_no_flat_part_carries_is_refused() -> Result<(), Box<dyn Error>> {
    // DV_CODED_TEXT.defining_code/terminology_id holds a TERMINOLOGY_ID, and a
    // manual value is written as text, so the write would not decode. The
    // compiler refuses it at load; this is the engine's backstop.
    let carrier = compiled("ferrobridge_tail")?;
    let node = mapping(&carrier, "certainty")
        .and_then(Mapping::openehr)
        .ok_or("the certainty node")?;
    let linked = Manual::new(
        String::from("linked"),
        Vec::new(),
        vec![ManualPath::new(
            Target::Openehr(Box::new(with_tail(node, "defining_code/terminology_id")?)),
            ManualValue::Literal(String::from("http://example.org/ferrobridge/certainty")),
        )],
        None,
        None,
        None,
        None,
    );
    let program = uncompiled(Mapping::new(MappingParts {
        manual: vec![linked],
        ..value_parts("certainty", condition_target("$resource")?, node.clone())?
    }))?;
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
    .expect_err("a manual value is no TERMINOLOGY_ID");
    assert!(
        matches!(
            error,
            EngineError::UnsupportedTail { ref mapping, ref tail, .. }
                if mapping == "certainty.linked" && tail.contains("terminology_id")
        ),
        "the refusal names the manual entry and the tail: {error}"
    );
    Ok(())
}
