// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legacy half of the `v2` root set: `hl7v2_types::legacy` against the
//! fetched IGAMT tables and HL7 table 0354.
//!
//! The agreement tests read each table as plain JSON, apart from the
//! generator's own projection, and compare it with the lowered model and the
//! emitted statics.
#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::missing_assert_message,
    reason = "test assertions: the helpers index plain JSON and fail the test that calls them"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fhir_codegen::v2::legacy::lower::{LegacyDefect, LegacyError, LegacyModel};
use fhir_codegen::v2::legacy::source::{
    CodeStatus, TABLE_0354, TABLES_DIR, Table0354, Tables, version_key,
};
use hl7v2_types::model::{DataTypeRef, Max, Node, Structure};
use serde_json::Value;

use crate::v2::MODEL;
use crate::vendor_dir;

fn legacy_dir() -> PathBuf {
    vendor_dir().join("hl7-v2-legacy")
}

fn tables_dir() -> PathBuf {
    legacy_dir().join(TABLES_DIR)
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    Tables::open(legacy_dir())
        .expect("the legacy tables should be fetched (scripts/vendor/v2-legacy.sh) and load")
});

static TABLE: LazyLock<Table0354> =
    LazyLock::new(|| Table0354::open(vendor_dir()).expect("table 0354 loads"));

static LEGACY: LazyLock<LegacyModel> =
    LazyLock::new(|| LegacyModel::lower(&TABLES, &MODEL, &TABLE).expect("the legacy tables lower"));

fn raw(version: &str, table: &str) -> Vec<Value> {
    let text = fs::read_to_string(tables_dir().join(version).join(format!("{table}.json")))
        .expect("table readable");
    serde_json::from_str::<Vec<Value>>(&text).expect("a JSON array")
}

/// The structure codes of the v2.9.1 definitions, variants aside.
fn current_codes() -> BTreeSet<String> {
    MODEL
        .structures
        .keys()
        .map(|id| match id.rsplit_once('-') {
            Some((base, variant)) if variant.len() == 1 => base.to_owned(),
            _ => id.clone(),
        })
        .collect()
}

#[test]
fn the_root_set_is_every_rooted_structure_the_v2_9_1_definitions_lack() {
    let current = current_codes();
    let mut expected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for version in TABLES.versions() {
        let groups = raw(&version.version, "groups");
        for message in raw(&version.version, "messages") {
            let id = message["id"].as_str().expect("id");
            let rooted = groups
                .iter()
                .any(|group| group["message_id"] == id && group["is_root"] == true);
            if rooted && !current.contains(id) {
                expected
                    .entry(id.to_owned())
                    .or_default()
                    .insert(version.version.clone());
            }
        }
    }
    let mut lowered: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for version in &LEGACY.versions {
        for id in version.structures.keys() {
            lowered
                .entry(id.clone())
                .or_default()
                .insert(version.version.clone());
        }
    }
    assert_eq!(lowered, expected);
    assert_eq!(expected.len(), 57);
    assert_eq!(LEGACY.structure_count(), 218);
    assert_eq!(
        hl7v2_types::legacy::STRUCTURES.len(),
        LEGACY.structure_count()
    );
    for structure in &hl7v2_types::legacy::STRUCTURES {
        assert!(
            !current.contains(structure.id),
            "{} is carried by v2.9.1",
            structure.id
        );
        assert_eq!(structure.url, None);
        assert!(structure.withdrawn_as_of.is_some(), "{}", structure.id);
    }
}

#[test]
fn orm_o01_has_a_tree_per_version_it_is_listed_in_and_is_withdrawn_as_of_2_7() {
    let versions: Vec<&str> = hl7v2_types::legacy::versions("ORM_O01")
        .iter()
        .map(|structure| structure.version)
        .collect();
    assert_eq!(versions, ["2.3", "2.3.1", "2.4", "2.5", "2.5.1", "2.6"]);
    for structure in hl7v2_types::legacy::versions("ORM_O01") {
        assert_eq!(structure.withdrawn_as_of, Some("2.7"));
    }
    let insurance = |structure: &Structure| {
        fn find(nodes: &'static [Node]) -> Option<Max> {
            nodes.iter().find_map(|node| match node {
                Node::Group(group) if group.name == "INSURANCE" => Some(group.cardinality.max),
                Node::Group(group) => find(group.children),
                _ => None,
            })
        }
        find(structure.nodes)
    };
    let at = |version: &str| {
        hl7v2_types::legacy::find("ORM_O01", version).expect("ORM_O01 is carried at the version")
    };
    assert_eq!(insurance(at("2.3")), Some(Max::Bounded(1)));
    assert_eq!(insurance(at("2.3.1")), Some(Max::Unbounded));
    assert!(hl7v2_types::legacy::find("ORM_O01", "2.7").is_none());
    assert!(hl7v2_types::structure::find("ORM_O01").is_none());
}

#[test]
fn every_withdrawn_as_of_is_the_first_version_that_lists_the_structure_no_more() {
    let mut order: Vec<&str> = TABLES
        .versions()
        .iter()
        .map(|version| version.version.as_str())
        .collect();
    order.push("2.9.1");
    for structure in &hl7v2_types::legacy::STRUCTURES {
        let last = TABLES
            .versions()
            .iter()
            .rposition(|version| {
                raw(&version.version, "messages")
                    .iter()
                    .any(|message| message["id"] == structure.id)
            })
            .expect("listed somewhere");
        assert_eq!(
            structure.withdrawn_as_of,
            order.get(last + 1).copied(),
            "{}",
            structure.id
        );
    }
}

#[test]
fn every_legacy_code_is_deprecated_in_table_0354_or_listed() {
    let codes: BTreeSet<&str> = hl7v2_types::legacy::STRUCTURES
        .iter()
        .map(|structure| structure.id)
        .collect();
    let mut listed = 0;
    for code in codes {
        let location = format!("{TABLE_0354}#{code}");
        match TABLE.status(code) {
            CodeStatus::Stated("deprecated") => {}
            CodeStatus::Stated("active") => {
                assert!(
                    LegacyDefect::ActiveInTable0354.is_tolerated(&location),
                    "{code}"
                );
                listed += 1;
            }
            CodeStatus::Absent => {
                assert!(
                    LegacyDefect::NotInTable0354.is_tolerated(&location),
                    "{code}"
                );
                listed += 1;
            }
            other => panic!("{code}: {other:?}"),
        }
    }
    assert_eq!(listed, 7);
    assert_eq!(TABLE.status("ORM_O01"), CodeStatus::Stated("deprecated"));
}

#[test]
fn every_tolerated_legacy_defect_is_met_where_it_is_listed() {
    let defects = [
        LegacyDefect::RepeatedPosition,
        LegacyDefect::MessageWithoutTree,
        LegacyDefect::UnreachedGroup,
        LegacyDefect::ZxxSlot,
        LegacyDefect::SegmentWithoutFields,
        LegacyDefect::OptionalChoiceMember,
        LegacyDefect::NotInTable0354,
        LegacyDefect::ActiveInTable0354,
    ];
    for defect in defects {
        for scope in defect.tolerated_in() {
            assert!(
                LEGACY
                    .tolerated
                    .iter()
                    .any(|(met, location)| *met == defect && location == scope),
                "{defect:?} is listed for {scope} but not met there"
            );
        }
    }
    assert!(
        LEGACY
            .tolerated
            .iter()
            .all(|(met, location)| met.is_tolerated(location))
    );
}

#[test]
fn the_message_index_links_each_legacy_entry_to_its_versioned_structure() {
    let entries = hl7v2_types::message::legacy("ORM", "O01");
    let versions: Vec<&str> = entries.iter().map(|entry| entry.version).collect();
    assert_eq!(versions, ["2.3", "2.3.1", "2.4", "2.5", "2.5.1", "2.6"]);
    let entry = hl7v2_types::message::find_legacy("ORM", "O01", "2.5.1").expect("ORM^O01 2.5.1");
    assert!(std::ptr::eq(
        entry.structure,
        &raw const hl7v2_types::legacy::v2_5_1::structure::orm_o01::ORM_O01
    ));
    assert!(hl7v2_types::message::find("ORM", "O01").is_none());
    assert!(hl7v2_types::message::find_legacy("ORM", "O01", "2.7").is_none());
    assert_eq!(hl7v2_types::message::LEGACY.len(), LEGACY.structure_count());
    let keys: Vec<(&str, &str, Vec<u32>)> = hl7v2_types::message::LEGACY
        .iter()
        .map(|entry| {
            (
                entry.code,
                entry.event,
                version_key(entry.version).expect("a dotted version"),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "the index is in (code, event, version) order");
}

/// Every segment reference in `nodes`, depth first.
fn references(nodes: &'static [Node], out: &mut Vec<&'static hl7v2_types::model::Segment>) {
    for node in nodes {
        match node {
            Node::Segment(reference) => out.push(reference.segment),
            Node::Group(group) => references(group.children, out),
            Node::Placeholder(_) => {}
        }
    }
}

#[test]
fn a_tree_links_an_agreeing_segment_to_the_v2_9_1_static_and_emits_the_others() {
    let mut shared = 0;
    let mut own = 0;
    for structure in &hl7v2_types::legacy::STRUCTURES {
        let mut segments = Vec::new();
        references(structure.nodes, &mut segments);
        for segment in segments {
            if segment.url.is_some() {
                let current = hl7v2_types::segment::find(segment.id).expect("a v2.9.1 segment");
                assert!(std::ptr::eq(segment, current), "{}", segment.id);
                shared += 1;
            } else {
                assert!(
                    segment
                        .fields
                        .iter()
                        .all(|field| !matches!(field.data_type, Some(DataTypeRef::Defined(_)))),
                    "{} links no v2.9.1 data type",
                    segment.id
                );
                own += 1;
            }
        }
    }
    assert!(shared > 0 && own > 0, "{shared} shared, {own} own");
    assert_eq!(LEGACY.segment_count(), 495);
    assert_eq!(LEGACY.shared_count(), 98);
}

#[test]
fn lowered_legacy_fields_agree_with_the_field_and_data_element_tables() {
    for version in &LEGACY.versions {
        let fields = raw(&version.version, "fields");
        let elements: BTreeMap<String, Value> = raw(&version.version, "data_elements")
            .into_iter()
            .map(|element| (element["id"].as_str().expect("id").to_owned(), element))
            .collect();
        for (id, segment) in &version.segments {
            let mut rows: Vec<&Value> = fields
                .iter()
                .filter(|field| field["segment_id"] == id.as_str())
                .collect();
            rows.sort_by_key(|field| {
                field["position"]
                    .as_str()
                    .and_then(|text| text.parse::<u16>().ok())
            });
            assert_eq!(rows.len(), segment.fields.len(), "{} {id}", version.version);
            for (row, field) in rows.iter().zip(&segment.fields) {
                let context = format!("{} {}", version.version, field.id);
                let element = &elements[row["data_element_id"].as_str().expect("an item")];
                assert_eq!(row["position"], field.position.to_string(), "{context}");
                assert_eq!(element["description"], field.name.as_str(), "{context}");
                let code = element["datatype_id"].as_str().expect("a data type");
                assert_eq!(
                    field.data_type.as_deref(),
                    (code != "-").then_some(code),
                    "{context}"
                );
                assert_eq!(
                    element["table_id"].as_str(),
                    field.table.as_ref().map(|table| table.id.as_str()),
                    "{context}"
                );
                assert_eq!(row["min"], field.cardinality.min.to_string(), "{context}");
            }
        }
    }
}

/// A copy of one version's tables and the provenance, for a test to damage.
fn copy_version(version: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::copy(
        legacy_dir().join("PROVENANCE.md"),
        dir.path().join("PROVENANCE.md"),
    )
    .expect("provenance copied");
    let target = dir.path().join(TABLES_DIR).join(version);
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
    let copy = copy_version("2.3");
    let elements = copy.path().join(TABLES_DIR).join("2.3/elements.json");
    let root = raw("2.3", "groups")
        .into_iter()
        .find(|group| group["message_id"] == "ORM_O01" && group["is_root"] == true)
        .expect("the ORM_O01 root")["id"]
        .clone();
    damage(&elements, |rows| {
        let mut siblings = rows
            .iter_mut()
            .filter(|row| row["parent_id"] == root)
            .collect::<Vec<_>>();
        siblings.sort_by_key(|row| row["position"].as_u64());
        let first = siblings[0]["position"].clone();
        siblings[1]["position"] = first;
    });
    let tables = Tables::open(copy.path()).expect("the copy loads");
    match LegacyModel::lower(&tables, &MODEL, &TABLE) {
        Err(LegacyError::Defect {
            location, defect, ..
        }) => {
            assert_eq!(location, "2.3/elements.json");
            assert_eq!(defect, LegacyDefect::RepeatedPosition);
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
