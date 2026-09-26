// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The defects refused on a damaged copy of the tables.

use std::fs;
use std::path::Path;

use fhir_codegen::v2::legacy::lower::{LegacyDefect, LegacyError, LegacyModel};
use fhir_codegen::v2::legacy::source::{TABLE_0354, TABLES_DIR, Table0354, Tables};
use serde_json::Value;

use crate::v2::MODEL;
use crate::v2_legacy::TABLE;
use crate::v2_legacy::TABLES;
use crate::v2_legacy::legacy_dir;
use crate::v2_legacy::raw;
use crate::v2_legacy::tables_dir;
use crate::vendor_dir;

/// A copy of one version's tables and the provenance, for a test to damage.
fn copy_version(version: &str) -> tempfile::TempDir {
    copy_version_as(version, version)
}

/// A copy of the tables of `version` under the directory `name`.
fn copy_version_as(version: &str, name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::copy(
        legacy_dir().join("PROVENANCE.md"),
        dir.path().join("PROVENANCE.md"),
    )
    .expect("provenance copied");
    let target = dir.path().join(TABLES_DIR).join(name);
    fs::create_dir_all(&target).expect("dir created");
    for entry in fs::read_dir(tables_dir().join(version)).expect("listable") {
        let path = entry.expect("entry").path();
        fs::copy(&path, target.join(path.file_name().expect("a name"))).expect("copied");
    }
    dir
}

fn damage(path: &Path, change: impl FnOnce(&mut Vec<Value>)) {
    let mut rows: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(path).expect("readable")).expect("JSON");
    change(&mut rows);
    fs::write(path, serde_json::to_string(&rows).expect("serializes")).expect("written");
}

#[test]
fn a_repeated_position_outside_its_files_is_refused() {
    // Every elements.json of 2.3 to 2.8.2 repeats a position, so the 2.3 tables are read as
    // 2.2, whose only listed defect is its messages.json, with each position set to the row id
    // (the order siblings take) before the one repeat is made.
    let copy = copy_version_as("2.3", "2.2");
    let elements = copy.path().join(TABLES_DIR).join("2.2/elements.json");
    let root = raw("2.3", "groups")
        .into_iter()
        .find(|group| group["message_id"] == "ORM_O01" && group["is_root"] == true)
        .expect("the ORM_O01 root")["id"]
        .clone();
    let mut damaged = String::new();
    damage(&elements, |rows| {
        for row in rows.iter_mut() {
            row["position"] = row["id"].clone();
        }
        let mut siblings = rows
            .iter_mut()
            .filter(|row| row["parent_id"] == root)
            .collect::<Vec<_>>();
        siblings.sort_by_key(|row| row["id"].as_u64());
        let first = siblings[0]["position"].clone();
        siblings[1]["position"] = first;
        damaged = siblings[1]["id"].to_string();
    });
    let tables = Tables::open(copy.path()).expect("the copy loads");
    match LegacyModel::lower(&tables, &MODEL, &TABLE) {
        Err(LegacyError::Defect {
            location,
            row,
            defect,
        }) => {
            assert_eq!(location, "2.2/elements.json");
            assert_eq!(defect, LegacyDefect::RepeatedPosition);
            assert_eq!(row, damaged, "the damaged element is the one refused");
        }
        other => panic!("expected a RepeatedPosition refusal, got {other:?}"),
    }
}

#[test]
fn a_structure_table_0354_marks_active_outside_its_list_is_refused() {
    let vendor = tempfile::tempdir().expect("tempdir");
    let path = vendor.path().join(TABLE_0354);
    fs::create_dir_all(path.parent().expect("a parent")).expect("dir created");
    let mut system: Value =
        serde_json::from_str(&fs::read_to_string(vendor_dir().join(TABLE_0354)).expect("readable"))
            .expect("JSON");
    for concept in system["concept"].as_array_mut().expect("concepts") {
        if concept["code"] == "ORM_O01" {
            for property in concept["property"].as_array_mut().expect("properties") {
                if property["code"] == "status" {
                    property["valueCode"] = Value::from("active");
                }
            }
        }
    }
    fs::write(&path, serde_json::to_string(&system).expect("serializes")).expect("written");
    let table = Table0354::open(vendor.path()).expect("the copy loads");
    match LegacyModel::lower(&TABLES, &MODEL, &table) {
        Err(LegacyError::Defect {
            location, defect, ..
        }) => {
            assert_eq!(location, format!("{TABLE_0354}#ORM_O01"));
            assert_eq!(defect, LegacyDefect::ActiveInTable0354);
        }
        other => panic!("expected an ActiveInTable0354 refusal, got {other:?}"),
    }
}

#[test]
fn an_unknown_usage_code_is_refused() {
    let copy = copy_version("2.3");
    let fields = copy.path().join(TABLES_DIR).join("2.3/fields.json");
    damage(&fields, |rows| {
        let orc = rows
            .iter_mut()
            .find(|row| row["segment_id"] == "ORC")
            .expect("an ORC field");
        orc["usage"] = Value::from("Q");
    });
    let tables = Tables::open(copy.path()).expect("the copy loads");
    match LegacyModel::lower(&tables, &MODEL, &TABLE) {
        Err(LegacyError::Invalid { file, reason, .. }) => {
            assert_eq!(file, "2.3/fields.json");
            assert_eq!(reason, "usage \"Q\"");
        }
        other => panic!("expected an Invalid refusal, got {other:?}"),
    }
}

#[test]
fn a_directory_not_named_as_a_version_is_refused() {
    let copy = copy_version("2.3");
    fs::create_dir_all(copy.path().join(TABLES_DIR).join("latest")).expect("dir created");
    assert!(matches!(
        Tables::open(copy.path()),
        Err(fhir_codegen::v2::legacy::source::SourceError::NotAVersion { ref name }) if name == "latest"
    ));
}
