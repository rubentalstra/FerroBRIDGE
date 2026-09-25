// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first FHIR round trip: the published KDS diagnosis chain of the
//! FHIRconnect mapping library under a FerroBRIDGE project context, compiled
//! against the published `KDS_Diagnose` operational template.
//!
//! The published files load verbatim from the vendored library. Where one
//! does not load or compile strictly, its refusal is asserted here by file
//! and line and the project directory carries a stand-in that says what it
//! departs from; the library defects are recorded on #101 and #173. The two
//! lens laws run over the synthetic diagnosis chain of the testkit and over
//! the KDS chain.

use core::error::Error;
use std::path::Path;
use std::sync::Arc;

use ferrobridge_testkit::conformance::Case;
use ferrobridge_testkit::conformance::Corpus;
use ferrobridge_testkit::conformance::record;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::load::MappingSet;
use fhirconnect::model::load::load_set;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::Program;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::index::AqlPath;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::template::TemplateSource;

use crate::laws;
use crate::support::FIXTURES;
use crate::support::compiled;
use crate::support::render;
use crate::support::template;

/// The vendored mapping library, relative to this crate's manifest.
const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/fhirconnect-mapping-lib"
);

/// The published files the project context loads, as the library ships them.
const PUBLISHED: &[&str] = &[
    "model/evaluation/org.openehr/problem_diagnosis.v1.yml",
    "model/cluster/org.openehr/anatomical_location.v1.yml",
    "model/cluster/org.openehr/problem_qualifier.v2.yml",
    "model/cluster/org.highmed/multiple_coding_icd10gm.v1.yml",
    "model/cluster/org.openehr/case_identification.v0.yml",
    "model/composition/org.openehr/report.v1.Condition.yml",
    "projects/org.highmed/KDS/diagnose/KDS_problem_qualifier.yml",
    "projects/org.highmed/KDS/diagnose/KDS_anatomical_location.yml",
    "projects/org.highmed/KDS/diagnose/KDS_lebensphase.yml",
];

/// The project directory this crate authored for the round trip.
const PROJECT_DIR: &str = "projects/ferrobridge/kds_diagnose";

/// The files of the project directory, the context first.
const PROJECT: &[&str] = &[
    "ferrobridge_kds_diagnose.context.yml",
    "ferrobridge_kds_problem_diagnose.yml",
    "ferrobridge_kds_problem_qualifier.yml",
    "ferrobridge_lebensphase.v0.yml",
    "ferrobridge_kds_composition.Condition.yml",
];

/// The context this crate authored over the chain.
const CONTEXT: &str = "ferrobridge_kds_diagnose.context";

/// The published diagnosis extension the strict loader refuses.
const KDS_PROBLEM_DIAGNOSE: &str = "projects/org.highmed/KDS/diagnose/KDS_problem_diagnose.yml";

/// Returns the absolute path of a file of the vendored library.
fn published(file: &str) -> String {
    format!("{CORPUS}/{file}")
}

/// Returns the absolute path of a file of the project directory.
fn project(file: &str) -> String {
    format!("{FIXTURES}/{PROJECT_DIR}/{file}")
}

/// Builds the index over the published `KDS_Diagnose` template.
pub(crate) fn kds_template() -> Result<WebTemplateIndex, Box<dyn Error>> {
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT)?;
    Ok(WebTemplateIndex::build(&TemplateSource::Opt14(Box::new(
        opt,
    )))?)
}

/// Loads the published files and the project directory into one set.
fn kds_set() -> Result<MappingSet, Box<dyn Error>> {
    let files: Vec<String> = PUBLISHED
        .iter()
        .map(|file| published(file))
        .chain(PROJECT.iter().map(|file| project(file)))
        .collect();
    load_set(files, &StaticMappingCodes::default())
        .map_err(|diagnostics| render(&diagnostics).into())
}

/// Compiles the project context against the published template.
fn kds_program() -> Result<(Arc<Program>, WebTemplateIndex), Box<dyn Error>> {
    let index = kds_template()?;
    let set = kds_set()?;
    let program = compile(
        &set,
        &MappingName::new(CONTEXT)?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .map_err(|diagnostics| render(&diagnostics))?;
    Ok((program, index))
}

/// Compiles the project context and returns it with the published files it
/// loads, relative to the vendored library.
pub(crate) fn kds_chain() -> Result<crate::support::Chain, Box<dyn Error>> {
    let (program, _) = kds_program()?;
    Ok((PUBLISHED, program))
}

/// Returns the refusals a load of `files` raises, as `file:line code` rows.
fn refusals(files: &[String]) -> Result<Vec<String>, Box<dyn Error>> {
    match load_set(files, &StaticMappingCodes::default()) {
        Ok(_) => Err("the files are expected not to load".into()),
        Err(diagnostics) => Ok(rows(&diagnostics)),
    }
}

/// Renders diagnostics as sorted `file:line code` rows, the file by its name.
fn rows(diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut found: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| {
            let file = diagnostic
                .file()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let line = diagnostic
                .position()
                .map_or(0, openehr_mapping_core::position::Position::line);
            format!("{file}:{line} {}", diagnostic.code())
        })
        .collect();
    found.sort();
    found
}

#[test]
fn the_kds_project_context_compiles_against_the_published_template() -> Result<(), Box<dyn Error>> {
    let (program, _) = kds_program()?;
    assert_eq!(program.template().id().as_str(), "KDS_Diagnose");
    assert_eq!(program.resource().as_str(), "Condition");
    insta::assert_snapshot!("kds_program", program.to_string());
    Ok(())
}

#[test]
fn the_published_files_the_context_reaches_load_strictly() -> Result<(), Box<dyn Error>> {
    // The one project file the set needs to load them is the lebensphase
    // model stand-in, which KDS_lebensphase extends.
    let files: Vec<String> = PUBLISHED
        .iter()
        .map(|file| published(file))
        .chain([project("ferrobridge_lebensphase.v0.yml")])
        .collect();
    let set = load_set(files, &StaticMappingCodes::default()).map_err(|d| render(&d))?;
    for name in [
        "EVALUATION.problem_diagnosis.v1",
        "CLUSTER.anatomical_location.v1",
        "CLUSTER.problem_qualifier.v2",
        "CLUSTER.multiple_coding_icd10gm.v1",
        "CLUSTER.case_identification.v0",
        "COMPOSITION.report.v1.Condition",
        "KDS_problem_qualifier",
        "KDS_anatomical_location",
        "KDS_lebensphase",
    ] {
        assert!(
            set.model(&MappingName::new(name)?).is_some(),
            "{name} loaded"
        );
    }
    Ok(())
}

/// `KDS_problem_diagnose.yml` line 100 writes `"extensionValue"#TODO` with no
/// white space before the `#`, which YAML 1.2.2 §6.6 does not admit as a
/// comment. Recorded on #101.
#[test]
fn the_published_kds_problem_diagnose_is_refused_at_its_line_100() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        refusals(&[published(KDS_PROBLEM_DIAGNOSE)])?,
        vec!["KDS_problem_diagnose.yml:100 yaml-syntax"]
    );
    Ok(())
}

/// `lebensphase.v0.yml` line 15 writes `mappings:` with no value, and the
/// published model schema types `mappings` as an array. Recorded on #101.
#[test]
fn the_published_lebensphase_model_is_refused_for_its_null_mappings() -> Result<(), Box<dyn Error>>
{
    let found = refusals(&[published("model/cluster/org.highmed/lebensphase.v0.yml")])?;
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found
            .first()
            .is_some_and(|row| row.starts_with("lebensphase.v0.yml:")
                && row.ends_with(" fc-schema-violation")),
        "{found:?}"
    );
    Ok(())
}

/// `KDS_composition.yml` line 9 extends `COMPOSITION.report_result.v1.Condition`;
/// the library ships `COMPOSITION.report.v1.Condition`. Recorded on #101.
#[test]
fn the_published_kds_composition_extends_a_name_no_model_carries() -> Result<(), Box<dyn Error>> {
    let files = [
        published("model/composition/org.openehr/report.v1.Condition.yml"),
        published("model/cluster/org.openehr/case_identification.v0.yml"),
        published("projects/org.highmed/KDS/diagnose/KDS_composition.yml"),
    ];
    let Err(diagnostics) = load_set(&files, &StaticMappingCodes::default()) else {
        return Err("the published composition extension is expected not to load".into());
    };
    assert_eq!(
        rows(&diagnostics),
        vec!["KDS_composition.yml:9 fc-unknown-mapping-reference"]
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message()
            .contains("`COMPOSITION.report_result.v1.Condition`")),
        "{}",
        render(&diagnostics)
    );
    Ok(())
}

/// Compiling the chain as the library publishes it, with only the comment of
/// `KDS_problem_diagnose.yml` line 100 separated from its token, refuses
/// exactly the rows below. The published context itself cannot be used,
/// because its composition extension does not load, so the project context
/// lists the four published extensions and this crate's composition
/// extension in its place.
///
/// - `KDS_problem_diagnose.yml:32`: `extension` on a nested method (#101).
/// - `KDS_problem_diagnose.yml:241`: the reference slots the start model into
///   itself.
/// - `KDS_anatomical_location.yml:15`: `value` read on `CodeableConcept` under
///   the published `bodySite` slot (#173).
/// - `anatomical_location.v1.yml:33`: `Condition.coding` (#173).
/// - `multiple_coding_icd10gm.v1.yml:30, 56, 95`: `value` read on a `Coding`
///   (#101).
/// - `problem_diagnosis.v1.yml:60, 71`: `$archetype/diagnose/...` (#101).
/// - `problem_diagnosis.v1.yml:103`: `data[at0012]`, which `KDS_Diagnose`
///   does not carry and the archetype's data structure is not.
/// - `report.v1.Condition.yml:92, 94, 96`: `null_flavour` below the
///   `DV_DATE_TIME` of `context/start_time`, in the second of two methods
///   named `contextStartTimeEmpty`, which no overwrite can reach (#101).
/// - `report.v1.Condition.yml:74` and `KDS_problem_diagnose.yml:100`: a
///   method name a sibling already carries (#101).
#[test]
fn the_kds_chain_as_published_is_refused_exactly() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let original = std::fs::read_to_string(published(KDS_PROBLEM_DIAGNOSE))?;
    let separated = original.replacen("\"extensionValue\"#TODO", "\"extensionValue\" #TODO", 1);
    assert_ne!(original, separated, "the line 100 comment is where it was");
    let diagnose = directory.path().join("KDS_problem_diagnose.yml");
    std::fs::write(&diagnose, separated)?;
    let context = std::fs::read_to_string(project("ferrobridge_kds_diagnose.context.yml"))?
        .replace(
            "    - \"ferrobridge_kds_problem_diagnose\"\n",
            "    - \"KDS_problem_diagnose\"\n",
        )
        .replace(
            "    - \"ferrobridge_kds_problem_qualifier\"\n",
            "    - \"KDS_problem_qualifier\"\n",
        )
        .replace(
            "    - \"KDS_lebensphase\"\n",
            "    - \"KDS_lebensphase\"\n    - \"KDS_anatomical_location\"\n",
        );
    let context_file = directory.path().join("published.context.yml");
    std::fs::write(&context_file, context)?;
    let files: Vec<String> = PUBLISHED
        .iter()
        .map(|file| published(file))
        .chain([
            project("ferrobridge_lebensphase.v0.yml"),
            project("ferrobridge_kds_composition.Condition.yml"),
            diagnose.to_string_lossy().into_owned(),
            context_file.to_string_lossy().into_owned(),
        ])
        .collect();
    let set = load_set(files, &StaticMappingCodes::default()).map_err(|d| render(&d))?;
    let diagnostics = compile(
        &set,
        &MappingName::new(CONTEXT)?,
        &kds_template()?,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .err()
    .ok_or("the chain as published compiles")?;
    insta::assert_json_snapshot!("kds_chain_as_published", rows(&diagnostics));
    Ok(())
}

/// Returns the synthetic Condition of the testkit with a date of onset.
fn synthetic_condition() -> Result<serde_json::Value, Box<dyn Error>> {
    let mut resource: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = resource.as_object_mut() {
        object.insert(
            String::from("onsetDateTime"),
            serde_json::Value::String(String::from("2026-09-12T09:00:00+02:00")),
        );
    }
    Ok(resource)
}

/// Builds a composition of the synthetic diagnosis template, the problem
/// name and its date of onset.
fn synthetic_composition(index: &WebTemplateIndex) -> Result<CanonicalComposition, Box<dyn Error>> {
    let root = "/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]";
    let values = vec![
        NodeValue::new(
            index.node(&AqlPath::new("/composer"))?,
            "FerroBRIDGE".into(),
        )
        .with_datum("name"),
        NodeValue::new(index.node(&AqlPath::new("/language"))?, "de".into()).with_datum("code"),
        NodeValue::new(index.node(&AqlPath::new("/territory"))?, "DE".into()).with_datum("code"),
        NodeValue::new(
            index.node(&AqlPath::new(format!("{root}/items[at0002]/value")))?,
            "a synthetic problem name".into(),
        )
        .with_datum("value"),
        NodeValue::new(
            index.node(&AqlPath::new(format!("{root}/items[at0077]/value")))?,
            "2026-09-12T09:00:00+02:00".into(),
        ),
    ];
    Ok(index.build_composition(&values, laws::NOW)?)
}

#[test]
fn putget_holds_on_the_synthetic_diagnosis_chain() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let declared = laws::putget(
        &program,
        &template()?,
        &synthetic_condition()?,
        &laws::untouched,
    )?;
    insta::assert_json_snapshot!("synthetic_putget", declared);
    Ok(())
}

#[test]
fn getput_holds_on_the_synthetic_diagnosis_chain() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let declared = laws::getput(
        &program,
        &index,
        &synthetic_composition(&index)?,
        &laws::untouched,
    )?;
    insta::assert_json_snapshot!("synthetic_getput", declared);
    Ok(())
}

#[test]
fn a_coding_flipped_between_the_putget_legs_breaks_the_law() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let resource = synthetic_condition()?;
    let clean = laws::putget(&program, &index, &resource, &laws::untouched)?;
    let flipped = laws::putget(&program, &index, &resource, &|composition| {
        ferrobridge_testkit::laws::replace_first(composition, "SYN-001", "SYN-FLIPPED")
    })?;
    assert_ne!(clean, flipped, "a changed coding went unnoticed");
    assert_eq!(
        flipped["changed"],
        serde_json::json!(["code.coding[0].code: \"SYN-001\" became \"SYN-FLIPPED\""]),
        "the flipped coding is named: {flipped}"
    );
    Ok(())
}

#[test]
fn a_value_flipped_between_the_getput_legs_breaks_the_law() -> Result<(), Box<dyn Error>> {
    let program = compiled("ferrobridge_diagnose_minimal")?;
    let index = template()?;
    let composition = synthetic_composition(&index)?;
    let clean = laws::getput(&program, &index, &composition, &laws::untouched)?;
    let flipped = laws::getput(&program, &index, &composition, &|resource| {
        ferrobridge_testkit::laws::replace_first(
            resource,
            "a synthetic problem name",
            "another name",
        )
    })?;
    assert_ne!(clean, flipped, "a changed value went unnoticed");
    Ok(())
}

/// Returns the synthetic Condition shaped like the MII Diagnose profile.
fn kds_condition() -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(serde_json::from_str(
        ferrobridge_testkit::fixtures::KDS_DIAGNOSE_CONDITION,
    )?)
}

/// Builds the synthetic `KDS_Diagnose` composition from its FLAT fixture.
fn kds_composition(index: &WebTemplateIndex) -> Result<CanonicalComposition, Box<dyn Error>> {
    let text = std::fs::read_to_string(Path::new(&project("kds_diagnose.flat.json")))?;
    let flat: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&text)?;
    Ok(index.build_from_flat(&flat, laws::NOW)?)
}

#[test]
fn the_kds_flat_fixture_builds_a_composition_the_template_admits() -> Result<(), Box<dyn Error>> {
    let index = kds_template()?;
    let composition = kds_composition(&index)?;
    assert_eq!(composition.template_id(), "KDS_Diagnose");
    Ok(())
}

#[test]
fn putget_holds_on_the_kds_condition() -> Result<(), Box<dyn Error>> {
    let (program, index) = kds_program()?;
    let declared = laws::putget(&program, &index, &kds_condition()?, &laws::untouched)?;
    insta::assert_json_snapshot!("kds_putget", declared);
    Ok(())
}

#[test]
fn getput_holds_on_the_kds_composition() -> Result<(), Box<dyn Error>> {
    let (program, index) = kds_program()?;
    let declared = laws::getput(
        &program,
        &index,
        &kds_composition(&index)?,
        &laws::untouched,
    )?;
    insta::assert_json_snapshot!("kds_getput", declared);
    Ok(())
}

/// Returns the declared set a reviewed snapshot of this file pins.
fn reviewed(name: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    let text = std::fs::read_to_string(format!(
        "{}/tests/it/snapshots/it__roundtrip__{name}.snap",
        env!("CARGO_MANIFEST_DIR")
    ))?;
    let body = text
        .splitn(3, "---\n")
        .nth(2)
        .ok_or_else(|| format!("the {name} snapshot has no body"))?;
    Ok(serde_json::from_str(body)?)
}

/// Returns the verdict on one law run: it holds when the run completes and
/// its declared set is the reviewed one.
fn holds(
    law: &str,
    snapshot: &str,
    run: Result<serde_json::Value, Box<dyn Error>>,
) -> Result<Option<String>, Box<dyn Error>> {
    let expected = reviewed(snapshot)?;
    Ok(match run {
        Err(error) => Some(format!("{law} did not run: {error}")),
        Ok(declared) if declared == expected => None,
        Ok(declared) => Some(format!(
            "{law} declared another set than the reviewed one: {declared}"
        )),
    })
}

/// Returns the verdict on one chain from its two laws.
fn chain_case(id: &str, putget: Option<String>, getput: Option<String>) -> Case {
    match putget.or(getput) {
        Some(reason) => Case::fail(id, reason),
        None => Case::pass(id),
    }
}

/// The conformance verdict on every round-trip chain.
///
/// A chain passes when both lens laws hold on it: `PutGet` over its FHIR
/// resource and `GetPut` over its composition, each equal to its input modulo
/// the reviewed declared set. No specification governs this: our own design.
#[test]
fn conformance_the_round_trip_chains_hold_their_pass_list() -> Result<(), Box<dyn Error>> {
    let synthetic = {
        let program = compiled("ferrobridge_diagnose_minimal")?;
        let index = template()?;
        let putget = laws::putget(&program, &index, &synthetic_condition()?, &laws::untouched);
        let getput = laws::getput(
            &program,
            &index,
            &synthetic_composition(&index)?,
            &laws::untouched,
        );
        chain_case(
            "ferrobridge_diagnose_minimal",
            holds("PutGet", "synthetic_putget", putget)?,
            holds("GetPut", "synthetic_getput", getput)?,
        )
    };
    let kds = {
        let (program, index) = kds_program()?;
        let putget = laws::putget(&program, &index, &kds_condition()?, &laws::untouched);
        let getput = laws::getput(
            &program,
            &index,
            &kds_composition(&index)?,
            &laws::untouched,
        );
        chain_case(
            "kds_diagnose",
            holds("PutGet", "kds_putget", putget)?,
            holds("GetPut", "kds_getput", getput)?,
        )
    };
    let outcome = record(Corpus::Roundtrip, &[synthetic, kds])?;
    assert!(
        outcome.regressed.is_empty(),
        "cases the pass list records no longer pass: {:?}",
        outcome.regressed
    );
    Ok(())
}
