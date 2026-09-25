// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL mapping library, run through every validation layer.
//!
//! The library is evidence, never an oracle: OMOCL's railroad images and
//! syntax tables decide what a file may contain, and these tests pin where the
//! library departs from them. Every expectation below is an exact set, so the
//! day upstream changes a file the test fails and the file is re-adjudicated.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Write as _;

use omocl::model::ast::EntityType;
use omocl::model::load::load_file;
use omocl::model::load::load_set;
use omocl::model::projection::Attestation;
use omocl::model::projection::KEYS_WITHOUT_COLUMN;
use omocl::model::projection::projection;
use omocl::model::semantic::FirstPartyConverters;
use openehr_mapping_core::loader;

use crate::support::corpus_files;
use crate::support::corpus_path;

/// How many mapping files the vendored corpus holds.
const CORPUS_SIZE: usize = 208;

/// Every corpus file FerroBRIDGE refuses: the file, the code of the first
/// refusal, its line, how many refusals the file raises, and the defect.
const SKIPS: &[(&str, &str, u64, usize, &str)] = &[
    (
        "medical_data/cluster/Inspired_oxygen_v1.yml",
        "omocl-missing-key",
        33,
        1,
        "`operator_concept_id` (line 32) writes `optional` and no `alternatives`; the \
         railroad's CONVERSION clause opens with `alternatives`",
    ),
    (
        "medical_data/evaluation/Hand_dominance_v1.yml",
        "yaml-syntax",
        22,
        1,
        "line 22 indents with a tab; YAML 1.2.2 §6.1: tab characters must not be used in \
         indentation",
    ),
    (
        "medical_data/observation/Demo_v1.yml",
        "malformed-path",
        23,
        1,
        "the path writes `items/[at0012]`, a predicate with no attribute, which the openEHR \
         path grammar (BASE Release 1.2.0, Paths and Locators) does not admit",
    ),
    (
        "medical_data/observation/Esas_r_v1.yml",
        "omocl-key-without-column",
        24,
        10,
        "ten `Measurement` records write `qualifier`; CDM v5.4 MEASUREMENT has no qualifier \
         column",
    ),
    (
        "medical_data/observation/Medication_screening_v1.yml",
        "malformed-path",
        47,
        1,
        "the path ends in a space after `items[at0024]`, which the openEHR path grammar does \
         not admit",
    ),
    (
        "medical_data/observation/Menstrual_diary_v1.yml",
        "yaml-duplicate-key",
        27,
        1,
        "one `conceptMap` `mapping` writes the key `at0005` five times, so four of its five \
         concept mappings would be discarded",
    ),
    (
        "medical_data/observation/Nine_hole_peg_test_v1.yml",
        "omocl-key-without-column",
        28,
        4,
        "four `Measurement` records write `qualifier`; CDM v5.4 MEASUREMENT has no qualifier \
         column",
    ),
    (
        "medical_data/observation/Paced_auditory_serial_addition_test_v1.yml",
        "omocl-key-without-column",
        24,
        2,
        "two `Measurement` records write `qualifier`; CDM v5.4 MEASUREMENT has no qualifier \
         column",
    ),
    (
        "medical_data/observation/Urinalysis_v1.yml",
        "omocl-key-without-column",
        103,
        1,
        "a `Measurement` record writes `qualifier`; CDM v5.4 MEASUREMENT has no qualifier \
         column",
    ),
];

/// Every corpus file whose `metadata.name` differs from its file stem, with
/// the name it declares.
///
/// Neither OMOCL nor the shared header ties the name to the file name, so the
/// loader accepts all eight; they are recorded here as data.
/// `Person_v1.yml` declares a name that is another archetype's.
const NAME_MISMATCHES: &[(&str, &str)] = &[
    (
        "medical_data/cluster/Figo_staging_cancer.yml",
        "Figo_staging_cancer_v1",
    ),
    ("medical_data/cluster/Fnclcc.yml", "Fnclcc_v1"),
    (
        "medical_data/cluster/Gist_modified_nih.yml",
        "Gist_modified_nih_v1",
    ),
    ("medical_data/cluster/Person_v1.yml", "CLUSTER.address.v1"),
    (
        "medical_data/cluster/Who_grade_bone_sarcoma.yml",
        "Who_grade_bone_sarcoma_v1",
    ),
    (
        "medical_data/evaluation/alcohol_consumption_summary_v1.yml",
        "Alcohol_consumption_summary_v1",
    ),
    (
        "medical_data/observation/Body_segment_length_v1.yml",
        "Body_segment_length",
    ),
    ("person_data/person_data_v0.yml", "person_data.v0"),
];

/// The six files the corpus refresh to `c082db8e` added, with whether each
/// loads.
const REFRESH_FILES: &[(&str, bool)] = &[
    ("medical_data/admin_entry/Citizenship_v1.yml", true),
    ("medical_data/evaluation/Hand_dominance_v1.yml", false),
    ("medical_data/observation/Berg_balance_scale_v1.yml", true),
    ("medical_data/observation/Guss_icu_v1.yml", true),
    ("medical_data/observation/Guss_v1.yml", true),
    (
        "medical_data/observation/Physical_activity_screening_v1.yml",
        true,
    ),
];

/// The keys an entry writes that are no column key.
const STRUCTURAL_KEYS: &[&str] = &["type", "base_path", "archetype_id", "name"];

/// Every YAML-readable corpus file the authored schema refuses on its own.
///
/// It is the skip set less the two YAML refusals, which never reach the
/// schema, and less the two malformed paths, whose openEHR path grammar a JSON
/// Schema string type does not carry.
const SCHEMA_REJECTIONS: &[&str] = &[
    "medical_data/cluster/Inspired_oxygen_v1.yml",
    "medical_data/observation/Esas_r_v1.yml",
    "medical_data/observation/Nine_hole_peg_test_v1.yml",
    "medical_data/observation/Paced_auditory_serial_addition_test_v1.yml",
    "medical_data/observation/Urinalysis_v1.yml",
];

#[test]
fn the_authored_schema_refuses_exactly_the_grammar_skips() -> Result<(), Box<dyn Error>> {
    let schema = omocl::model::schema::schema().map_err(ToString::to_string)?;
    let mut refused = Vec::new();
    for relative in corpus_files()? {
        let Ok(document) = loader::load_file(corpus_path(&relative)) else {
            continue;
        };
        if !schema.validate_document(&document).is_empty() {
            refused.push(relative);
        }
    }
    assert_eq!(refused, SCHEMA_REJECTIONS, "the schema's verdict moved");
    Ok(())
}

#[test]
fn every_corpus_file_loads_or_is_an_adjudicated_skip() -> Result<(), Box<dyn Error>> {
    let files = corpus_files()?;
    assert_eq!(files.len(), CORPUS_SIZE, "the corpus size moved");
    let mut refused = Vec::new();
    for relative in &files {
        if let Err(diagnostics) = load_file(corpus_path(relative), &FirstPartyConverters) {
            let first = diagnostics
                .first()
                .ok_or("a refusal carries a diagnostic")?;
            for diagnostic in &diagnostics {
                assert!(
                    diagnostic.file().ends_with(relative),
                    "a diagnostic names the file it is about: {diagnostic}"
                );
                assert!(diagnostic.position().is_some(), "{diagnostic}");
            }
            refused.push((
                relative.clone(),
                first.code().to_string(),
                first
                    .position()
                    .map_or(0, openehr_mapping_core::position::Position::line),
                diagnostics.len(),
            ));
        }
    }
    let expected: Vec<(String, String, u64, usize)> = SKIPS
        .iter()
        .map(|(file, code, line, count, _)| ((*file).to_owned(), (*code).to_owned(), *line, *count))
        .collect();
    assert_eq!(refused, expected, "the adjudicated skip set moved");
    Ok(())
}

#[test]
fn the_loadable_corpus_loads_as_one_set() -> Result<(), Box<dyn Error>> {
    let skipped: BTreeSet<&str> = SKIPS.iter().map(|(file, ..)| *file).collect();
    let files: Vec<_> = corpus_files()?
        .into_iter()
        .filter(|relative| !skipped.contains(relative.as_str()))
        .map(|relative| corpus_path(&relative))
        .collect();
    let set = load_set(&files, &FirstPartyConverters)
        .map_err(|diagnostics| crate::support::render(&diagnostics))?;
    assert_eq!(set.len(), CORPUS_SIZE - SKIPS.len());
    Ok(())
}

#[test]
fn the_name_mismatches_are_recorded_and_not_refused() -> Result<(), Box<dyn Error>> {
    let mut found = Vec::new();
    for relative in corpus_files()? {
        let Ok(document) = loader::load_file(corpus_path(&relative)) else {
            continue;
        };
        let stem = corpus_path(&relative)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("a file stem")?
            .to_owned();
        let name = document.header().name().value().to_string();
        if name != stem {
            found.push((relative, name));
        }
    }
    let expected: Vec<(String, String)> = NAME_MISMATCHES
        .iter()
        .map(|(file, name)| ((*file).to_owned(), (*name).to_owned()))
        .collect();
    assert_eq!(found, expected);
    let skipped: BTreeSet<&str> = SKIPS.iter().map(|(file, ..)| *file).collect();
    for (file, _) in NAME_MISMATCHES {
        assert!(!skipped.contains(file), "{file} is refused for its name");
    }
    Ok(())
}

#[test]
fn the_six_refreshed_files_load_or_are_skips() {
    let skipped: BTreeSet<&str> = SKIPS.iter().map(|(file, ..)| *file).collect();
    for (file, loads) in REFRESH_FILES {
        let outcome = load_file(corpus_path(file), &FirstPartyConverters);
        assert_eq!(outcome.is_ok(), *loads, "{file}");
        assert_eq!(skipped.contains(file), !*loads, "{file}");
    }
}

/// Derives the key union per entry `type` over every file the YAML loader
/// reads.
fn key_union() -> Result<BTreeMap<String, BTreeSet<String>>, Box<dyn Error>> {
    let mut union: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for relative in corpus_files()? {
        let Ok(document) = loader::load_file(corpus_path(&relative)) else {
            continue;
        };
        let Some(entries) = document
            .document()
            .get("mappings")
            .and_then(|m| m.as_sequence())
        else {
            continue;
        };
        for entry in entries {
            let Some(keys) = entry.as_mapping() else {
                continue;
            };
            let entity_type = entry
                .get("type")
                .and_then(|value| value.as_text())
                .ok_or_else(|| format!("{relative}: an entry names no type"))?;
            union
                .entry(entity_type.to_owned())
                .or_default()
                .extend(keys.keys().cloned());
        }
    }
    Ok(union)
}

#[test]
fn the_key_union_of_the_corpus_is_the_committed_snapshot() -> Result<(), Box<dyn Error>> {
    let mut rendered = String::new();
    for (entity_type, keys) in key_union()? {
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        writeln!(rendered, "{entity_type}: {}", keys.join(", "))?;
    }
    insta::assert_snapshot!("corpus_key_union", rendered);
    Ok(())
}

#[test]
fn every_key_the_corpus_writes_is_projected_or_recorded() -> Result<(), Box<dyn Error>> {
    for (entity_type, keys) in key_union()? {
        let parsed: EntityType = entity_type.parse()?;
        let EntityType::Target(target) = parsed else {
            for key in &keys {
                assert!(
                    STRUCTURAL_KEYS.contains(&key.as_str()),
                    "{entity_type}.{key}"
                );
            }
            continue;
        };
        for key in keys {
            let known = STRUCTURAL_KEYS.contains(&key.as_str())
                || projection(target).key(&key).is_some()
                || KEYS_WITHOUT_COLUMN
                    .iter()
                    .any(|entry| entry.target == target && entry.key == key);
            assert!(
                known,
                "the corpus writes `{entity_type}.{key}`, which nothing projects"
            );
        }
    }
    Ok(())
}

#[test]
fn every_library_attested_key_is_one_the_corpus_writes() -> Result<(), Box<dyn Error>> {
    let union = key_union()?;
    for target in omocl::model::ast::Target::ALL {
        let written = union
            .get(target.as_str())
            .ok_or("every target is written")?;
        for key in projection(*target).keys {
            let from_library = matches!(key.attestation, Attestation::Library | Attestation::Both);
            assert_eq!(
                written.contains(key.key),
                from_library,
                "`{target}.{}` is attested {:?}",
                key.key,
                key.attestation
            );
        }
    }
    Ok(())
}
