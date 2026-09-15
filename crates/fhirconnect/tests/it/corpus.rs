// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect mapping library, run through all three validation layers.
//!
//! The library is evidence, never an oracle: the FHIRconnect specification
//! decides what a mapping file may contain, and these tests pin where the
//! published files and the published schemas disagree with it. Every
//! expectation below is an exact set, so the day upstream changes either one
//! the test fails and the disagreement is re-adjudicated rather than absorbed.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use fhirconnect::model::error::SchemaKind;
use fhirconnect::model::load::load_set;
use fhirconnect::model::schema::Schemas;
use fhirconnect::model::schema::published;
use fhirconnect::model::schema::strict;
use fhirconnect::model::semantic::StaticMappingCodes;
use openehr_mapping_core::loader::load_str;

/// The vendored mapping library, relative to this crate's manifest.
const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/fhirconnect-mapping-lib"
);

/// The vendored FHIRconnect specification source, relative to the manifest.
const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/specs/fhirconnect");

/// How many mapping files the vendored corpus holds.
const CORPUS_SIZE: usize = 107;

/// Every corpus file the published schemas refuse, with its error count.
///
/// Three classes, all of them defects in the published model schema rather
/// than in the files: `#/$defs/mapping` sets `additionalProperties: false`
/// and omits `link` (the LINKED mapping) and `mappingCode` (the PROGRAMMED
/// mapping), both of which the prose defines
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`),
/// and three files write `mappings` with no value, which FerroBRIDGE refuses
/// too.
const PUBLISHED_REJECTIONS: &[(&str, usize)] = &[
    ("model/action/org.openehr/medication.v1.yml", 2),
    ("model/admin_entry/org.highmed/person_data.v0.yml", 1),
    ("model/cluster/org.highmed/lebensphase.v0.yml", 1),
    ("model/cluster/org.highmed/study_details.v1.yml", 1),
    ("model/cluster/org.openehr/dosage.v2.BackboneElement.yml", 1),
    ("model/cluster/org.openehr/dosage.v2.yml", 4),
    (
        "model/cluster/org.openehr/laboratory_test_analyte.v1.yml",
        1,
    ),
    ("model/cluster/org.openehr/specimen.v1.yml", 1),
    ("model/cluster/org.openehr/timing_daily.v1.yml", 1),
    ("model/cluster/org.openehr/timing_non_daily.yml", 1),
    ("model/evaluation/org.openehr/vital_status.v1.yml", 5),
    (
        "model/observation/org.openehr/laboratory_test_result.v1.yml",
        1,
    ),
    (
        "model/observation/org.openehr/medication_statement.v0.yml",
        3,
    ),
    ("projects/org.highmed/KDS/diagnose/KDS_composition.yml", 1),
    (
        "projects/org.highmed/KDS/laborauftrag/KDS_composition.yml",
        1,
    ),
    (
        "projects/org.highmed/KDS/laborbericht/KDS_composition.yml",
        1,
    ),
    (
        "projects/org.highmed/KDS/medikationseintrag/KDS_composition.yml",
        1,
    ),
    (
        "projects/org.highmed/KDS/medikationsverabreichung/KDS_composition.yml",
        1,
    ),
    (
        "projects/org.highmed/KDS/person/KDS_admin_entry_person.yml",
        1,
    ),
    (
        "projects/org.highmed/KDS/person_pseudo/KDS_pseudo_admin_person.yml",
        1,
    ),
    ("projects/org.highmed/KDS/procedure/KDS_composition.yml", 1),
    (
        "projects/org.highmed/KDS/studienteilnahme/KDS_informed_consent.yaml",
        1,
    ),
    (
        "projects/org.highmed/KDS/todesursache/KDS_composition.yml",
        1,
    ),
    (
        "projects/org.highmed/KDS/vitalstatus/KDS_composition.yml",
        1,
    ),
];

/// Every corpus file this crate's strict schemas refuse, with its reason.
///
/// The header page calls `metadata.name` "a unique id used to identify the
/// mapping and reference it"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Metadata)
/// and the published model schema types `mappings` as an array, so a
/// `mappings` written with no value is a file defect on both readings and
/// FerroBRIDGE refuses it.
const STRICT_REJECTIONS: &[&str] = &[
    "model/cluster/org.highmed/lebensphase.v0.yml",
    "model/cluster/org.highmed/study_details.v1.yml",
    "projects/org.highmed/KDS/studienteilnahme/KDS_informed_consent.yaml",
];

/// Every PROGRAMMED function the corpus names, in `mappingCode` order.
const CORPUS_MAPPING_CODES: &[&str] = &[
    "dosageDurationToAdministrationDuration",
    "dosageQuantityToRange",
    "ehrStatusAddExternalREF",
    "rangeToText",
    "ratio_to_dosage",
    "ratio_to_dosage_action",
    "timingNonDaily",
    "timingToDaily",
];

/// Returns every mapping file of the corpus, by path relative to its root.
fn corpus_files() -> Result<Vec<String>, Box<dyn Error>> {
    let root = PathBuf::from(CORPUS);
    let mut found = Vec::new();
    collect(&root, &root, &mut found)?;
    found.sort();
    Ok(found)
}

/// Walks `directory`, collecting every YAML file relative to `root`.
fn collect(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, found)?;
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "yml" || extension == "yaml");
        if is_yaml {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            found.push(relative);
        }
    }
    Ok(())
}

/// Compiles a pair of published schemas from a vendored attachments directory.
fn schemas_from(directory: &str) -> Result<Schemas, Box<dyn Error>> {
    let attachments = PathBuf::from(SPEC).join(directory);
    let model = fs::read_to_string(attachments.join(SchemaKind::Model.file_name()))?;
    let context = fs::read_to_string(attachments.join(SchemaKind::Context.file_name()))?;
    Ok(published::compile(&model, &context)?)
}

/// Returns this crate's own compiled schemas.
fn strict_schemas() -> Result<&'static Schemas, Box<dyn Error>> {
    strict::schemas().map_err(|error| error.to_string().into())
}

/// Validates the whole corpus against `schemas`, keyed by relative path.
fn validate_corpus(schemas: &Schemas) -> Result<BTreeMap<String, Vec<String>>, Box<dyn Error>> {
    let mut refused = BTreeMap::new();
    for relative in corpus_files()? {
        let source = fs::read_to_string(PathBuf::from(CORPUS).join(&relative))?;
        let Ok(document) = load_str(relative.clone(), &source) else {
            continue;
        };
        let diagnostics = schemas.validate_document(&document);
        if !diagnostics.is_empty() {
            refused.insert(
                relative,
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message().to_owned())
                    .collect(),
            );
        }
    }
    Ok(refused)
}

/// Returns every corpus file as an absolute path.
fn corpus_paths() -> Result<Vec<PathBuf>, Box<dyn Error>> {
    Ok(corpus_files()?
        .iter()
        .map(|relative| PathBuf::from(CORPUS).join(relative))
        .collect())
}

/// Runs the whole corpus through the loader and returns `file` and code pairs.
fn corpus_diagnostics(codes: &StaticMappingCodes) -> Result<Vec<String>, Box<dyn Error>> {
    let paths = corpus_paths()?;
    Ok(match load_set(&paths, codes) {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| {
                let file = diagnostic
                    .file()
                    .strip_prefix(CORPUS)
                    .unwrap_or_else(|_| diagnostic.file())
                    .to_string_lossy()
                    .replace('\\', "/");
                format!("{file}\t{}", diagnostic.code())
            })
            .collect(),
    })
}

#[test]
fn the_vendored_corpus_is_the_pinned_size() -> Result<(), Box<dyn Error>> {
    assert_eq!(corpus_files()?.len(), CORPUS_SIZE);
    Ok(())
}

#[test]
fn every_corpus_file_validates_against_the_published_schemas_or_is_in_the_pinned_set()
-> Result<(), Box<dyn Error>> {
    let refused = validate_corpus(&schemas_from("modules/ROOT/attachments")?)?;
    let measured: Vec<(String, usize)> = refused
        .iter()
        .map(|(file, errors)| (file.clone(), errors.len()))
        .collect();
    let expected: Vec<(String, usize)> = PUBLISHED_REJECTIONS
        .iter()
        .map(|(file, count)| ((*file).to_owned(), *count))
        .collect();
    assert_eq!(
        measured, expected,
        "the published schemas refuse a different set than the pinned one"
    );
    let total: usize = measured.iter().map(|(_, count)| *count).sum();
    assert_eq!(total, 34, "the pinned rejection set is 34 errors");
    assert_eq!(measured.len(), 24, "the pinned rejection set is 24 files");
    Ok(())
}

#[test]
fn the_published_rejections_fall_into_exactly_three_classes() -> Result<(), Box<dyn Error>> {
    let refused = validate_corpus(&schemas_from("modules/ROOT/attachments")?)?;
    let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
    for message in refused.values().flatten() {
        let class = if message.contains("'link' was unexpected") {
            "link"
        } else if message.contains("'mappingCode' was unexpected") {
            "mappingCode"
        } else if message.contains("null is not of type \"array\"") {
            "null mappings"
        } else {
            panic!("an unclassified published-schema rejection: {message}");
        };
        *classes.entry(class).or_default() += 1;
    }
    assert_eq!(classes.get("link").copied(), Some(22));
    assert_eq!(classes.get("mappingCode").copied(), Some(9));
    assert_eq!(classes.get("null mappings").copied(), Some(3));
    Ok(())
}

#[test]
fn the_two_vendored_copies_of_the_published_schemas_refuse_the_same_files()
-> Result<(), Box<dyn Error>> {
    let source = validate_corpus(&schemas_from("modules/ROOT/attachments")?)?;
    let rendered = validate_corpus(&schemas_from("build/site/FHIRconnect/v1.0.0/_attachments")?)?;
    assert_eq!(
        source, rendered,
        "the repository-source and rendered copies of the published schemas disagree over the \
         corpus"
    );
    Ok(())
}

#[test]
fn every_corpus_file_validates_against_the_strict_schemas_or_carries_an_adjudicated_skip()
-> Result<(), Box<dyn Error>> {
    let refused = validate_corpus(strict_schemas()?)?;
    let measured: Vec<&str> = refused.keys().map(String::as_str).collect();
    assert_eq!(measured, STRICT_REJECTIONS);
    for errors in refused.values() {
        for message in errors {
            assert!(
                message.contains("null is not of type \"array\""),
                "an unadjudicated strict-schema rejection: {message}"
            );
        }
    }
    Ok(())
}

#[test]
fn the_strict_schemas_accept_the_four_keys_the_published_model_schema_omits()
-> Result<(), Box<dyn Error>> {
    let published_refusals = validate_corpus(&schemas_from("modules/ROOT/attachments")?)?;
    let strict_refusals = validate_corpus(strict_schemas()?)?;
    let recovered: Vec<&String> = published_refusals
        .keys()
        .filter(|file| !strict_refusals.contains_key(*file))
        .collect();
    assert_eq!(
        recovered.len(),
        21,
        "the strict schemas recover the files the published ones refuse over `link` and \
         `mappingCode`"
    );
    Ok(())
}

/// The refusals are pinned in the order a load raises them, which is file
/// order through the loader and then document order through the semantic
/// rules, so a rule that starts reporting somewhere else is a failing test
/// rather than a silent reshuffle.
#[test]
fn the_corpus_semantic_refusals_are_the_pinned_set() -> Result<(), Box<dyn Error>> {
    let measured = corpus_diagnostics(&StaticMappingCodes::default())?;
    let expected = vec![
        "model/cluster/org.highmed/lebensphase.v0.yml\tfc-schema-violation".to_owned(),
        "model/cluster/org.highmed/study_details.v1.yml\tfc-schema-violation".to_owned(),
        "projects/org.highmed/KDS/diagnose/KDS_problem_diagnose.yml\tyaml-syntax".to_owned(),
        "projects/org.highmed/KDS/laborauftrag/KDS_composition.yml\tduplicate-mapping-name".to_owned(),
        "projects/org.highmed/KDS/person_pseudo/KDS_composition.yml\tduplicate-mapping-name".to_owned(),
        "projects/org.highmed/KDS/person_pseudo/pseudo_person.context.yaml\tduplicate-mapping-name".to_owned(),
        "projects/org.highmed/KDS/procedure/KDS_composition.yml\tduplicate-mapping-name".to_owned(),
        "projects/org.highmed/KDS/studienteilnahme/KDS_informed_consent.yaml\tfc-schema-violation".to_owned(),
        "projects/org.highmed/KDS/todesursache/KDS_composition.yml\tduplicate-mapping-name".to_owned(),
        "projects/org.highmed/KDS/vitalstatus/KDS_composition.yml\tduplicate-mapping-name".to_owned(),
        "projects/org.openehr/EEHRxF/lab/bundle/lab_composition.yml\tempty-document".to_owned(),
        "model/action/org.openehr/procedure.v1.yml\tfc-condition-target-root-mismatch".to_owned(),
        "model/cluster/org.openehr/dosage.v2.yml\tfc-unknown-mapping-code".to_owned(),
        "model/cluster/org.openehr/dosage.v2.yml\tfc-unknown-mapping-code".to_owned(),
        "model/cluster/org.openehr/dosage.v2.yml\tfc-unknown-mapping-code".to_owned(),
        "model/cluster/org.openehr/dosage.v2.yml\tfc-unknown-mapping-code".to_owned(),
        "model/cluster/org.openehr/dosage.v2.BackboneElement.yml\tfc-unknown-mapping-code".to_owned(),
        "model/cluster/org.highmed/study_participation.v1.yml\tfc-unknown-mapping-reference".to_owned(),
        "model/cluster/org.openehr/timing_daily.v1.yml\tfc-unknown-mapping-code".to_owned(),
        "model/cluster/org.openehr/timing_non_daily.yml\tfc-unknown-mapping-code".to_owned(),
        "model/composition/org.openehr/report-result.v1.Composition.yml\tfc-extension-method-outside-model".to_owned(),
        "model/composition/org.openehr/report.v1.MedicationAdministration.yml\tfc-condition-target-root-mismatch".to_owned(),
        "model/evaluation/org.openehr/cause_of_death.v1.yml\tfc-extension-method-outside-model".to_owned(),
        "projects/org.highmed/KDS/person_pseudo/KDS_pseudo_admin_person.yml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/person_pseudo/KDS_pseudo_admin_person.yml\tfc-unknown-mapping-code".to_owned(),
        "projects/org.highmed/KDS/person/KDS_admin_entry_person.yml\tfc-unknown-mapping-code".to_owned(),
        "projects/org.highmed/KDS/diagnose/KDS_composition.yml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/diagnose/KDS_lebensphase.yml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.openehr/EEHRxF/lab/bundle/lab_result.yml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/person/person.context.yaml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/studienteilnahme/studienteilnahme.context.yaml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/studienteilnahme/studienteilnahme.context.yaml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/diagnose/KDS_diagnose.context.yaml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.highmed/KDS/diagnose/KDS_diagnose.context.yaml\tfc-unknown-mapping-reference".to_owned(),
        "projects/org.openehr/EEHRxF/lab/bundle/lab.context.yml\tfc-unknown-mapping-reference".to_owned(),
    ];
    assert_eq!(measured, expected);
    Ok(())
}

#[test]
fn the_six_duplicate_mapping_names_are_refused_naming_both_files() -> Result<(), Box<dyn Error>> {
    let diagnostics = corpus_diagnostics(&StaticMappingCodes::default())?;
    let duplicates: Vec<&String> = diagnostics
        .iter()
        .filter(|entry| entry.ends_with("\tduplicate-mapping-name"))
        .collect();
    assert_eq!(
        duplicates.len(),
        6,
        "five files repeat `KDS_composition` and one repeats `KDS_Person.context`"
    );
    assert_eq!(
        duplicates
            .iter()
            .filter(|entry| entry.contains("KDS_composition.yml"))
            .count(),
        5,
        "{duplicates:?}"
    );
    Ok(())
}

#[test]
fn the_dangling_cross_references_are_refused_naming_the_files() -> Result<(), Box<dyn Error>> {
    let paths = corpus_paths()?;
    let Err(diagnostics) = load_set(&paths, &StaticMappingCodes::default()) else {
        return Err("the corpus is expected not to load clean".into());
    };
    let dangling: Vec<String> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "fc-unknown-mapping-reference")
        .map(|diagnostic| diagnostic.message().to_owned())
        .collect();
    assert_eq!(dangling.len(), 11, "{dangling:?}");
    for named in [
        "`COMPOSITION.report_result.v1.Condition`",
        "`COMPOSITION.person.v1`",
        "`KDS_person_data.v0`",
        "`COMPOSITION.report-result.v1.DiagnosticReport`",
    ] {
        assert!(
            dangling.iter().any(|message| message.contains(named)),
            "{named} is not among {dangling:?}"
        );
    }
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "fc-unknown-mapping-reference")
    {
        assert!(
            diagnostic.position().is_some(),
            "a dangling-reference diagnostic carries no position: {diagnostic}"
        );
        assert!(
            diagnostic.mapping_name().is_some(),
            "a dangling-reference diagnostic names no mapping: {diagnostic}"
        );
    }
    Ok(())
}

#[test]
fn the_corpus_mapping_codes_pass_once_the_engine_registers_them() -> Result<(), Box<dyn Error>> {
    let registered = StaticMappingCodes::new(CORPUS_MAPPING_CODES.iter().copied());
    let diagnostics = corpus_diagnostics(&registered)?;
    let unknown: Vec<&String> = diagnostics
        .iter()
        .filter(|entry| entry.ends_with("\tfc-unknown-mapping-code"))
        .collect();
    assert!(unknown.is_empty(), "{unknown:?}");
    let baseline = corpus_diagnostics(&StaticMappingCodes::default())?;
    assert_eq!(baseline.len() - diagnostics.len(), 9);
    Ok(())
}
