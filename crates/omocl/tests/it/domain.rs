// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The literal-concept domain check, over a stub vocabulary.
//!
//! The concept ids and domains below are synthetic stand-ins for a loaded
//! vocabulary, invented for the test; the hook takes whatever answers the
//! concept resolver gives.

use std::collections::BTreeMap;
use std::error::Error;

use omocl::model::ast::ConceptId;
use omocl::model::ast::DomainId;
use omocl::model::error::ModelCode;
use omocl::model::load::MappingSet;
use omocl::model::load::load_set;
use omocl::model::semantic::FirstPartyConverters;
use omocl::model::semantic::check_concept_domains;
use openehr_mapping_core::diagnostic::Diagnostic;

/// The header of the synthetic file, through line 11.
const HEADER: &str = "grammar: OMOCL/v1.0.0\ntype: model\nmetadata:\n  name: Domain_v1\n  \
                      version: 1.0.0\nspec:\n  system: OMOP\n  version: 5.4\n  \
                      openEhrConfig:\n    archetype: openEHR-EHR-OBSERVATION.domain.v1\n\
                      mappings:\n";

/// The stub vocabulary: concept id to domain.
fn vocabulary() -> BTreeMap<i32, &'static str> {
    BTreeMap::from([
        (1_000_001, "Measurement"),
        (1_000_002, "Condition"),
        (1_000_003, "Unit"),
        (1_000_004, "Meas Value"),
    ])
}

/// Loads the one-file set whose mappings are `body`.
fn set(body: &str) -> Result<MappingSet, Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("Domain_v1.yml");
    std::fs::write(&file, format!("{HEADER}{body}"))?;
    load_set([&file], &FirstPartyConverters)
        .map_err(|diagnostics| crate::support::render(&diagnostics).into())
}

/// Runs the check against the stub vocabulary.
fn check(set: &MappingSet) -> Vec<Diagnostic> {
    let vocabulary = vocabulary();
    let domain_of = |concept: ConceptId| {
        vocabulary
            .get(&concept.get())
            .map(|domain| DomainId::new(*domain))
    };
    check_concept_domains(set, &domain_of)
}

/// A `Measurement` record whose `concept_id` is the literal `code`.
fn measurement(code: i32) -> String {
    format!(
        "  - type: \"Measurement\"\n    concept_id:\n      alternatives:\n        - code: {code}\n"
    )
}

#[test]
fn a_literal_in_the_domain_of_its_column_passes() -> Result<(), Box<dyn Error>> {
    let diagnostics = check(&set(&measurement(1_000_001))?);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(())
}

#[test]
fn a_literal_of_another_domain_is_refused() -> Result<(), Box<dyn Error>> {
    let diagnostics = check(&set(&measurement(1_000_002))?);
    let first = diagnostics.first().ok_or("the literal passed")?;
    assert_eq!(first.code(), &ModelCode::DomainMismatch.into(), "{first}");
    assert!(first.message().contains("`Condition`"), "{first}");
    assert!(
        first
            .message()
            .contains("measurement.measurement_concept_id"),
        "{first}"
    );
    let position = first.position().ok_or("a position")?;
    assert_eq!((position.line(), position.column()), (15, 17));
    assert_eq!(
        first.model_path().to_string(),
        "mappings[0].concept_id.alternatives[0].code"
    );
    Ok(())
}

#[test]
fn a_literal_the_vocabulary_does_not_hold_is_refused() -> Result<(), Box<dyn Error>> {
    let diagnostics = check(&set(&measurement(42))?);
    let first = diagnostics.first().ok_or("the literal passed")?;
    assert_eq!(first.code(), &ModelCode::UnknownConcept.into(), "{first}");
    Ok(())
}

#[test]
fn the_no_matching_concept_is_exempt() -> Result<(), Box<dyn Error>> {
    let diagnostics = check(&set(&measurement(0))?);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(())
}

#[test]
fn a_unit_literal_is_checked_against_the_unit_domain() -> Result<(), Box<dyn Error>> {
    let body = format!(
        "{}    unit:\n      alternatives:\n        - path: \"/items[at0001]\"\n        - code: \
         1000001\n",
        measurement(1_000_001)
    );
    let diagnostics = check(&set(&body)?);
    let first = diagnostics.first().ok_or("the unit literal passed")?;
    assert_eq!(first.code(), &ModelCode::DomainMismatch.into(), "{first}");
    assert!(first.message().contains("`Unit`"), "{first}");
    Ok(())
}

#[test]
fn a_concept_map_value_is_checked_too() -> Result<(), Box<dyn Error>> {
    let body = format!(
        "{}    measurement_date:\n      alternatives:\n        - path: \"../context\"\n",
        measurement(1_000_001)
    )
    .replace(
        "        - code: 1000001\n",
        "        - conceptMap:\n            path: \"/items[at0002]\"\n            mapping:\n        \
         \x20     at0003: 1000001\n              at0004: 1000002\n",
    );
    let diagnostics = check(&set(&body)?);
    let codes: Vec<String> = diagnostics.iter().map(|d| d.code().to_string()).collect();
    assert_eq!(codes, ["omocl-domain-mismatch"], "{diagnostics:?}");
    let first = diagnostics.first().ok_or("one diagnostic")?;
    assert_eq!(
        first.model_path().to_string(),
        "mappings[0].concept_id.alternatives[0].conceptMap.mapping.at0004"
    );
    Ok(())
}

#[test]
fn a_column_the_cdm_names_no_domain_for_checks_existence_only() -> Result<(), Box<dyn Error>> {
    let body = "  - type: \"Observation\"\n    concept_id:\n      alternatives:\n        - code: \
                1000004\n";
    let diagnostics = check(&set(body)?);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let unknown = check(&set(&body.replace("1000004", "7"))?);
    let first = unknown.first().ok_or("the unknown literal passed")?;
    assert_eq!(first.code(), &ModelCode::UnknownConcept.into(), "{first}");
    Ok(())
}

#[test]
fn a_numeric_literal_under_a_non_concept_key_is_not_a_concept() -> Result<(), Box<dyn Error>> {
    let body = "  - type: \"Person\"\n    year_of_birth:\n      alternatives:\n        - code: 1990\n\
                \x20   gender_concept:\n      alternatives:\n        - code: 0\n";
    let diagnostics = check(&set(body)?);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(())
}
