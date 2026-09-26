// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The defects the lowering tolerates only where listed, and the ones it
//! refuses, on a damaged copy of the definitions.

use std::fs;
use std::path::Path;

use fhir_codegen::roots::{
    V2_BASES, V2_DATA_TYPE_DIRS, V2_MESSAGE_DIR, V2_SEGMENT_DIR, V2_STRUCTURE_DIR, V2RootSet,
};
use fhir_codegen::v2::corpus::{Corpus, LoadError, SOURCE_OF_TRUTH};
use fhir_codegen::v2::lower::{Defect, LowerError, Model};
use serde_json::Value;

use crate::v2::MODEL;
use crate::v2::definitions_dir;
use crate::v2::json_files;
use crate::v2::tree_dir;

#[test]
fn every_tolerated_defect_is_met_where_it_is_listed() {
    let defects = [
        Defect::NumberMax,
        Defect::TextInteger,
        Defect::MissingInteger,
        Defect::ConformanceLengthWithoutLength,
        Defect::ConformanceLengthWithoutNoTruncate,
        Defect::DashOptionality,
        Defect::LowercaseStatus,
        Defect::EmptyStatus,
        Defect::MinAboveMax,
        Defect::UndefinedDataType,
        Defect::EmptyStructure,
        Defect::PlaceholderGroupName,
        Defect::MisspelledDefinition,
        Defect::StructureProfileName,
        Defect::MessageWithoutStructure,
    ];
    for defect in defects {
        for scope in defect.tolerated_in() {
            assert!(
                MODEL.tolerated.iter().any(|(met, file)| *met == defect
                    && (file == scope || file.starts_with(&format!("{scope}/")))),
                "{defect:?} is listed for {scope} but not met there"
            );
        }
    }
    let listed = |(met, file): &(Defect, String)| met.is_tolerated(file);
    assert!(MODEL.tolerated.iter().all(listed));
}

/// A copy of the definitions the loader reads, for a test to damage.
fn copy_definitions() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::copy(
        definitions_dir().join("PROVENANCE.md"),
        dir.path().join("PROVENANCE.md"),
    )
    .expect("provenance copied");
    let target = dir.path().join(SOURCE_OF_TRUTH);
    let mut dirs = vec![V2_SEGMENT_DIR, V2_STRUCTURE_DIR, V2_MESSAGE_DIR];
    dirs.extend(V2_DATA_TYPE_DIRS);
    for sub in dirs {
        fs::create_dir_all(target.join(sub)).expect("dir created");
        for file in json_files(sub) {
            fs::copy(tree_dir().join(&file), target.join(&file)).expect("file copied");
        }
    }
    for base in V2_BASES {
        let path = target.join(base);
        fs::create_dir_all(path.parent().expect("a parent")).expect("dir created");
        fs::copy(tree_dir().join(base), path).expect("base copied");
    }
    dir
}

fn damage(root: &Path, file: &str, change: impl FnOnce(&mut Value)) {
    let path = root.join(SOURCE_OF_TRUTH).join(file);
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("readable")).expect("JSON");
    change(&mut value);
    fs::write(
        &path,
        serde_json::to_string_pretty(&value).expect("serializes"),
    )
    .expect("written");
}

fn lower_copy(root: &Path) -> Result<Model, LowerError> {
    let corpus = Corpus::open(root).expect("the copy loads");
    let roots = V2RootSet::select(&corpus).expect("roots");
    Model::lower(&corpus, &roots)
}

const OBX: &str = "segment/segments/OBX.json";

const ORU_R01_A: &str = "message-structure/message_structures/ORU_R01-A.json";

#[test]
fn a_number_max_outside_the_segment_directory_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), ORU_R01_A, |value| {
        value["differential"]["element"][1]["max"] = Value::from(1);
    });
    match lower_copy(copy.path()) {
        Err(LowerError::Defect {
            file,
            element,
            defect,
        }) => {
            assert_eq!(file, ORU_R01_A);
            assert_eq!(element, "ORU_R01-A.1-MSH");
            assert_eq!(defect, Defect::NumberMax);
        }
        other => panic!("expected a refused defect, got {other:?}"),
    }
}

#[test]
fn a_text_integer_outside_its_files_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["differential"]["element"][1]["extension"][1]["extension"][0]["valueInteger"] =
            Value::from("1");
    });
    assert!(matches!(
        lower_copy(copy.path()),
        Err(LowerError::Defect {
            defect: Defect::TextInteger,
            ..
        })
    ));
}

const ORU_R01: &str = "message/messages/ORU-R01.json";

const CX: &str = "data-type/complex/complex-data-types/cx.json";

fn refused_defect(copy: &Path, expected_file: &str, expected: Defect) {
    match lower_copy(copy) {
        Err(LowerError::Defect { file, defect, .. }) => {
            assert_eq!(file, expected_file);
            assert_eq!(defect, expected);
        }
        other => panic!("expected {expected:?} refused in {expected_file}, got {other:?}"),
    }
}

fn refused_invalid(copy: &Path, expected_file: &str, expected: &str) {
    match lower_copy(copy) {
        Err(LowerError::Invalid { file, reason, .. }) => {
            assert_eq!(file, expected_file);
            assert!(reason.contains(expected), "{reason}");
        }
        other => panic!("expected {expected:?} in {expected_file}, got {other:?}"),
    }
}

fn message_elements(value: &mut Value) -> &mut Vec<Value> {
    value["differential"]["element"]
        .as_array_mut()
        .expect("elements")
}

#[test]
fn a_message_without_a_structure_outside_its_files_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), ORU_R01, |value| {
        message_elements(value).retain(|element| element["id"] != "Message.structure");
    });
    refused_defect(copy.path(), ORU_R01, Defect::MessageWithoutStructure);
}

#[test]
fn a_structure_name_that_matches_no_structure_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), ORU_R01, |value| {
        for element in message_elements(value) {
            if element["id"] == "Message.structure" {
                element["type"][0]["targetProfile"][0] = Value::from(
                    "http://hl7.org/fhir/StructureDefinition/MessageStructure/ORU-R99-Z",
                );
            }
        }
    });
    refused_invalid(copy.path(), ORU_R01, "matches no message structure");
}

#[test]
fn a_trigger_event_the_id_does_not_name_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), ORU_R01, |value| {
        for element in message_elements(value) {
            if element["id"] == "Message.triggerEvent" {
                element["patternCode"] = Value::from("R02");
            }
        }
    });
    refused_invalid(copy.path(), ORU_R01, "is not the R01 the id names");
}

#[test]
fn an_unknown_message_element_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), ORU_R01, |value| {
        message_elements(value).push(serde_json::json!({
            "id": "Message.sender",
            "path": "Message.sender",
            "patternCode": "LAB"
        }));
    });
    refused_invalid(copy.path(), ORU_R01, "does not know");
}

#[test]
fn a_misspelled_definition_outside_the_complex_types_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["differential"]["element"][1]["defintion"] = Value::from("synthetic");
    });
    refused_defect(copy.path(), OBX, Defect::MisspelledDefinition);
}

#[test]
fn a_placeholder_group_name_outside_its_files_is_refused() {
    let copy = copy_definitions();
    let path = copy.path().join(SOURCE_OF_TRUTH).join(ORU_R01_A);
    let text = fs::read_to_string(&path).expect("readable");
    fs::write(
        &path,
        text.replace("ORU_R01-A.5-PATIENT_RESULT", "ORU_R01-A.5-FIXME"),
    )
    .expect("written");
    refused_defect(copy.path(), ORU_R01_A, Defect::PlaceholderGroupName);
}

#[test]
fn a_component_naming_no_data_type_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), CX, |value| {
        value["differential"]["element"][1]["type"][0]["code"] =
            Value::from("http://hl7.org/v2/StructureDefinition/NOPE");
    });
    refused_invalid(copy.path(), CX, "is no data type definition");
}

#[test]
fn a_component_out_of_order_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), CX, |value| {
        value["differential"]["element"][1]["id"] = Value::from("CX.2");
        value["differential"]["element"][1]["path"] = Value::from("CX.2");
    });
    refused_invalid(copy.path(), CX, "out of order");
}

fn refused_for(copy: &Path, expected: &str) {
    match lower_copy(copy) {
        Err(LowerError::Invalid { file, reason, .. }) => {
            assert_eq!(file, OBX);
            assert!(reason.contains(expected), "{reason}");
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

#[test]
fn an_unknown_extension_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["differential"]["element"][1]["extension"]
            .as_array_mut()
            .expect("extensions")
            .push(serde_json::json!({"url": "http://example.org/unknown", "valueCode": "Z"}));
    });
    refused_for(copy.path(), "unknown extension http://example.org/unknown");
}

#[test]
fn an_untyped_field_that_is_not_withdrawn_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["differential"]["element"][3]
            .as_object_mut()
            .expect("an element")
            .remove("type");
    });
    refused_for(copy.path(), "untyped field");
}

#[test]
fn an_unknown_optionality_code_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["differential"]["element"][1]["extension"][0]["valueCode"] = Value::from("Q");
    });
    refused_for(copy.path(), "optionality \"Q\"");
}

#[test]
fn a_snapshot_is_refused() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["snapshot"] = serde_json::json!({"element": []});
    });
    refused_for(copy.path(), "snapshot");
}

#[test]
fn an_unknown_element_member_is_refused_at_load() {
    let copy = copy_definitions();
    damage(copy.path(), OBX, |value| {
        value["differential"]["element"][1]["mustSupport"] = Value::from(true);
    });
    match Corpus::open(copy.path()) {
        Err(LoadError::Json { path, .. }) => assert!(path.ends_with(OBX)),
        other => panic!("expected a JSON refusal, got {other:?}"),
    }
}

#[test]
fn an_unfetched_tree_names_the_fetch_script() {
    let empty = tempfile::tempdir().expect("tempdir");
    let error = Corpus::open(empty.path()).expect_err("nothing to load");
    assert!(matches!(error, LoadError::NotFetched { .. }));
    assert!(error.to_string().contains("scripts/vendor/v2ig.sh"));
}
