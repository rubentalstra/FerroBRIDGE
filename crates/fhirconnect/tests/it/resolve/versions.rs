// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version selectors: the template, its semantic version, the profile
//! version and the archetype revision.

use core::error::Error;

use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::model::semantic::StaticMappingCodes;
use fhirconnect::resolve::compile::compile;
use fhirconnect::resolve::program::binding::Pin;
use fhirconnect::resolve::select::SelectError;
use fhirconnect::resolve::select::select_by_profile;
use openehr_mapping_core::header::metadata::MappingName;

use crate::resolve::CONTEXT;
use crate::resolve::EXTENSION;
use crate::resolve::PROBLEM;
use crate::resolve::borrow;
use crate::resolve::chain;
use crate::resolve::codes;
use crate::resolve::context;
use crate::resolve::model;
use crate::resolve::program::released_template;
use crate::resolve::program::versioned_template;
use crate::resolve::program_of;
use crate::resolve::refusals;
use crate::resolve::render;
use crate::resolve::revised_model;
use crate::resolve::set_of;
use crate::resolve::start_context;
use crate::resolve::start_model;

#[test]
fn a_template_the_context_does_not_name_is_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let declaration = context(
        "synthetic.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  template:\n    id: \"another.template.v1\"\n  archetypes:\n    - \"EVALUATION.synthetic.v1\"\n  start: \"EVALUATION.synthetic.v1\"\n",
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("context.yml", declaration),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-template-id-mismatch"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("another.template.v1"), "{message}");
    assert!(
        message.contains(ferrobridge_testkit::fixtures::DIAGNOSE_TEMPLATE_ID),
        "{message}"
    );
    Ok(())
}

#[test]
fn a_sem_ver_the_template_does_not_carry_is_recorded_unpinned() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    assert_eq!(program.template().sem_ver(), &Pin::Unpinned);
    assert!(program.template().sem_ver().is_unpinned());
    Ok(())
}

#[test]
fn a_sem_ver_disagreement_is_refused_naming_both_versions() -> Result<(), Box<dyn Error>> {
    let declaration = context(
        "synthetic.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  template:\n    id: \"ferrobridge.diagnose.v1\"\n    sem_ver: \"0.1.0\"\n  archetypes:\n    - \"EVALUATION.synthetic.v1\"\n  start: \"EVALUATION.synthetic.v1\"\n",
    );
    let files = [
        ("model.yml", start_model(PROBLEM)),
        ("context.yml", declaration),
    ];
    let set = set_of(&borrow(&files))?;
    let index = versioned_template("9.0.0-alpha.1")?;
    let name = MappingName::new("synthetic.context")?;
    let diagnostics = compile(
        &set,
        &name,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .err()
    .ok_or("a sem_ver disagreement is a refusal")?;
    assert_eq!(codes(&diagnostics), vec!["fc-template-sem-ver-mismatch"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("0.1.0"), "{message}");
    assert!(message.contains("9.0.0-alpha.1"), "{message}");
    Ok(())
}

#[test]
fn an_absent_profile_version_records_unpinned() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    assert_eq!(program.profile().version(), &Pin::Unpinned);
    Ok(())
}

#[test]
fn two_files_claiming_one_mapping_name_are_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let first = start_model(PROBLEM);
    let second = start_model(PROBLEM);
    let error = set_of(&[
        ("first.yml", first.as_str()),
        ("second.yml", second.as_str()),
    ])
    .err()
    .ok_or("two files claiming one name are refused")?
    .to_string();
    assert!(error.contains("first.yml"), "{error}");
    assert!(error.contains("second.yml"), "{error}");
    Ok(())
}

#[test]
fn the_program_records_the_version_of_every_model_it_compiled() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let program = program_of(&set, "ferrobridge_diagnose.context")
        .map_err(|diagnostics| render(&diagnostics))?;
    let start = program.models().first().ok_or("the start model")?;
    assert_eq!(start.version().as_str(), "0.0.1-alpha");
    assert_eq!(start.revision(), &Pin::Pinned(String::from("1.4.1")));
    assert_eq!(
        start.extensions(),
        [MappingName::new("ferrobridge_problem_diagnose")?]
    );
    Ok(())
}

#[test]
fn an_unresolved_archetype_root_is_refused_in_the_model_file() -> Result<(), Box<dyn Error>> {
    let files = [
        (
            "model.yml",
            model(
                "EVALUATION.synthetic.v1",
                "openEHR-EHR-EVALUATION.absent.v1",
                PROBLEM,
            ),
        ),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unresolved-archetype-root"]);
    let refusal = diagnostics.first().ok_or("one refusal")?;
    assert_eq!(refusal.file().to_string_lossy(), "model.yml");
    assert_eq!(
        refusal.mapping_name(),
        Some(&MappingName::new("EVALUATION.synthetic.v1")?)
    );
    Ok(())
}

#[test]
fn a_context_the_set_does_not_hold_is_refused_naming_it() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "absent.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-context"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("absent.context"), "{message}");
    Ok(())
}

/// The Web Template gives its root node the empty `aqlPath`, so a model
/// mapping rooted at the composition archetype anchors `$archetype` at the
/// root of the composition.
#[test]
fn a_model_mapping_rooted_at_the_composition_archetype_compiles() -> Result<(), Box<dyn Error>> {
    let body = "mappings:\n  - name: \"recorded\"\n    with:\n      fhir: \"$resource.recordedDate\"\n      openehr: \"$archetype/context/start_time\"\n";
    let composition = model(
        "COMPOSITION.synthetic.v1",
        "openEHR-EHR-COMPOSITION.ferrobridge_report.v1",
        body,
    );
    let declaration = context(
        "synthetic.context",
        "  profile:\n    url: \"http://example.org/fhir/StructureDefinition/synthetic\"\n  \
         template:\n    id: \"ferrobridge.diagnose.v1\"\n  archetypes:\n    - \
         \"COMPOSITION.synthetic.v1\"\n  start: \"COMPOSITION.synthetic.v1\"\n",
    );
    let set = set_of(&[
        ("model.yml", composition.as_str()),
        ("context.yml", declaration.as_str()),
    ])?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let mapping = program.mappings().first().ok_or("one mapping")?;
    let openehr = mapping.openehr().ok_or("the openEHR side")?;
    assert_eq!(openehr.path().to_string(), "/context/start_time");
    Ok(())
}

#[test]
fn an_absent_revision_records_unpinned() -> Result<(), Box<dyn Error>> {
    let files = [
        ("model.yml", start_model(PROBLEM)),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let set = set_of(&borrow(&files))?;
    let program =
        program_of(&set, "synthetic.context").map_err(|diagnostics| render(&diagnostics))?;
    let start = program.models().first().ok_or("the start model")?;
    assert_eq!(start.revision(), &Pin::Unpinned);
    Ok(())
}

#[test]
fn a_revision_disagreeing_with_the_template_is_refused_naming_both() -> Result<(), Box<dyn Error>> {
    let set = set_of(&[
        ("model.yml", revised_model("1.4.0", PROBLEM).as_str()),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])).as_str(),
        ),
    ])?;
    let index = released_template(
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1.4.1",
    )?;
    let diagnostics = compile(
        &set,
        &MappingName::new("synthetic.context")?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .err()
    .ok_or("a revision disagreement is a refusal")?;
    assert_eq!(codes(&diagnostics), vec!["fc-archetype-revision-mismatch"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("1.4.0"), "{message}");
    assert!(message.contains("1.4.1"), "{message}");
    Ok(())
}

#[test]
fn a_revision_the_template_carries_compiles_against_the_full_archetype_id()
-> Result<(), Box<dyn Error>> {
    let set = set_of(&[
        ("model.yml", revised_model("1.4.1", PROBLEM).as_str()),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])).as_str(),
        ),
    ])?;
    let index = released_template(
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1",
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1.4.1",
    )?;
    let program = compile(
        &set,
        &MappingName::new("synthetic.context")?,
        &index,
        &SCHEMAS,
        &StaticMappingCodes::default(),
    )
    .map_err(|diagnostics| render(&diagnostics))?;
    let start = program.models().first().ok_or("the start model")?;
    assert_eq!(start.revision(), &Pin::Pinned(String::from("1.4.1")));
    Ok(())
}

/// `fhirConfig.structureDefinition` is "just information for the user"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Spec) and
/// the specification names no other source for the resource type, so the
/// compiler needs it. No specification governs this: our own design.
#[test]
fn a_start_model_naming_no_structure_definition_is_refused() -> Result<(), Box<dyn Error>> {
    let unnamed = format!(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: EVALUATION.synthetic.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    archetype: openEHR-EHR-EVALUATION.problem_diagnosis.v1\n{PROBLEM}"
    );
    let files = [
        ("model.yml", unnamed),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-resource-type-unnamed"]);
    Ok(())
}

/// The last segment of the canonical URL is the resource name for a base
/// resource (<https://hl7.org/fhir/R4/structuredefinition.html>), so a URL
/// whose last segment names no resource type of this FHIR version is refused.
#[test]
fn a_structure_definition_naming_no_resource_type_is_refused() -> Result<(), Box<dyn Error>> {
    let unknown = format!(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: EVALUATION.synthetic.v1\n  version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    archetype: openEHR-EHR-EVALUATION.problem_diagnosis.v1\n  fhirConfig:\n    structureDefinition: http://hl7.org/fhir/StructureDefinition/Diagnosis\n{PROBLEM}"
    );
    let files = [
        ("model.yml", unknown),
        (
            "context.yml",
            context("synthetic.context", &start_context(&[])),
        ),
    ];
    let diagnostics = refusals(&borrow(&files), "synthetic.context")?;
    assert_eq!(codes(&diagnostics), vec!["fc-unknown-resource-type"]);
    let message = diagnostics.first().ok_or("one refusal")?.message();
    assert!(message.contains("Diagnosis"), "{message}");
    Ok(())
}

#[test]
fn a_profile_version_the_program_does_not_pin_is_refused() -> Result<(), Box<dyn Error>> {
    let set = chain(&[CONTEXT, EXTENSION]).map_err(|diagnostics| render(&diagnostics))?;
    let programs = vec![
        program_of(&set, "ferrobridge_diagnose.context")
            .map_err(|diagnostics| render(&diagnostics))?,
    ];
    let claimed = vec![String::from(
        "http://example.org/fhir/StructureDefinition/ferrobridge-diagnosis|2.0.0",
    )];
    let error = select_by_profile(&programs, &claimed)
        .err()
        .ok_or("the instance claims another version")?;
    match error {
        SelectError::ProfileVersion {
            wanted,
            ref mismatches,
        } => {
            assert!(wanted.contains("ferrobridge-diagnosis|2.0.0"), "{wanted}");
            assert_eq!(mismatches.len(), 1, "{mismatches:?}");
            let only = mismatches.first().ok_or("one mismatch")?;
            assert!(only.contains("claims `2.0.0`"), "{only}");
            assert!(only.contains("pins `1.0.0`"), "{only}");
        }
        other => return Err(format!("{other}").into()),
    }
    Ok(())
}
