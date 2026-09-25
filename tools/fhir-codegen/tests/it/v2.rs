// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `v2` root set: `hl7v2-types` against the fetched HL7 v2 definitions.
//!
//! The agreement tests read each definition file as plain JSON, apart from
//! the generator's own projection, and compare it with the emitted statics
//! element by element.
#![allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::missing_assert_message,
    reason = "test assertions: the helpers index plain JSON and fail the test that calls them"
)]

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fhir_codegen::roots::{
    V2_BASES, V2_DATA_TYPE_DIRS, V2_SEGMENT_DIR, V2_STRUCTURE_DIR, V2RootSet,
};
use fhir_codegen::v2::corpus::{Corpus, LoadError, SOURCE_OF_TRUTH};
use fhir_codegen::v2::emit::{EmitError, EmitOptions, emit};
use fhir_codegen::v2::lower::{Defect, LowerError, Model};
use fhir_codegen::v2::render::names;
use hl7v2_types::model::{
    Cardinality, ConditionalCode, GroupKind, Max, Node, Optionality, SegmentStatus, StandardsStatus,
};
use serde_json::Value;

use crate::vendor_dir;

fn definitions_dir() -> PathBuf {
    vendor_dir().join("hl7-v2ig")
}

fn tree_dir() -> PathBuf {
    definitions_dir().join(SOURCE_OF_TRUTH)
}

fn hl7v2_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/hl7v2-types")
}

/// The fetched definitions, loaded once.
static CORPUS: LazyLock<Corpus> = LazyLock::new(|| {
    Corpus::open(definitions_dir())
        .expect("the HL7 v2 definitions should be fetched (scripts/vendor/v2ig.sh) and load")
});

/// The lowered model, once.
static MODEL: LazyLock<Model> = LazyLock::new(|| {
    let roots = V2RootSet::select(&CORPUS).expect("every structure file is a message structure");
    Model::lower(&CORPUS, &roots).expect("the definitions lower")
});

fn raw(file: &str) -> Value {
    let text = fs::read_to_string(tree_dir().join(file)).expect("definition readable");
    serde_json::from_str(&text).expect("definition is JSON")
}

fn elements(value: &Value) -> Vec<Value> {
    value["differential"]["element"]
        .as_array()
        .expect("a differential")
        .clone()
}

fn json_files(dir: &str) -> Vec<String> {
    let mut files: Vec<String> = fs::read_dir(tree_dir().join(dir))
        .expect("dir listable")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "json"))
        .map(|name| format!("{dir}/{name}"))
        .collect();
    files.sort();
    files
}

fn raw_integer(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn raw_cardinality(element: &Value) -> Cardinality {
    let min = u32::try_from(element["min"].as_u64().expect("min")).expect("min fits");
    let max = match &element["max"] {
        Value::String(star) if star == "*" => Max::Unbounded,
        other => Max::Bounded(u32::try_from(raw_integer(other).expect("max")).expect("max fits")),
    };
    Cardinality { min, max }
}

fn extension<'a>(element: &'a Value, url: &str) -> Option<&'a Value> {
    element["extension"]
        .as_array()
        .and_then(|extensions| extensions.iter().find(|extension| extension["url"] == url))
}

fn nested<'a>(extension: &'a Value, url: &str) -> Option<&'a Value> {
    extension["extension"]
        .as_array()
        .and_then(|nested| nested.iter().find(|entry| entry["url"] == url))
        .map(|entry| &entry["valueInteger"])
}

fn conditional_code(code: ConditionalCode) -> &'static str {
    match code {
        ConditionalCode::R => "R",
        ConditionalCode::Re => "RE",
        ConditionalCode::O => "O",
        ConditionalCode::X => "X",
    }
}

fn optionality_code(optionality: Optionality) -> String {
    match optionality {
        Optionality::R => String::from("R"),
        Optionality::Re => String::from("RE"),
        Optionality::O => String::from("O"),
        Optionality::C => String::from("C"),
        Optionality::Conditional { first, second } => {
            format!(
                "C({}/{})",
                conditional_code(first),
                conditional_code(second)
            )
        }
        Optionality::X => String::from("X"),
        Optionality::B => String::from("B"),
        Optionality::W => String::from("W"),
        Optionality::Unstated => String::from("-"),
    }
}

#[test]
fn every_structure_file_and_every_segment_file_is_emitted() {
    let files = json_files(V2_STRUCTURE_DIR);
    assert_eq!(files.len(), 305);
    assert_eq!(hl7v2_types::structure::STRUCTURES.len(), files.len());
    for file in &files {
        let id = raw(file)["id"].as_str().expect("id").to_owned();
        let emitted = hl7v2_types::structure::find(&id).expect("every structure is emitted");
        assert_eq!(
            emitted.url,
            format!("http://hl7.org/v2/StructureDefinition/{id}")
        );
    }
    assert_eq!(hl7v2_types::segment::SEGMENTS.len(), MODEL.segments.len());
    assert_eq!(MODEL.segments.len(), 190);
    let segment_files = json_files(V2_SEGMENT_DIR);
    // Every segment file is a root, the Hxx slot file aside.
    assert_eq!(segment_files.len(), MODEL.segments.len() + 1);
    for file in &segment_files {
        let id = raw(file)["id"].as_str().expect("id").to_owned();
        let emitted = hl7v2_types::segment::find(&id);
        if id == "Hxx" {
            assert!(emitted.is_none(), "Hxx is a slot, never a segment");
        } else {
            assert!(emitted.is_some(), "{id} is emitted");
        }
    }
}

#[test]
fn the_batch_envelopes_and_the_unreferenced_segments_are_in_the_crate() {
    let reached: std::collections::BTreeSet<&str> = hl7v2_types::structure::STRUCTURES
        .iter()
        .flat_map(|structure| {
            let mut entries = Vec::new();
            segment_ids(structure.nodes, &mut entries);
            entries
        })
        .collect();
    for id in [
        "BHS", "BTS", "FHS", "FTS", "ADD", "FAC", "OVR", "PDC", "PSH",
    ] {
        let segment = hl7v2_types::segment::find(id).expect("an unreferenced segment is emitted");
        assert!(!segment.fields.is_empty(), "{id} carries its field table");
        assert!(!reached.contains(id), "no structure references {id}");
    }
    let bhs = hl7v2_types::segment::find("BHS").expect("BHS");
    assert_eq!(bhs.fields.first().map(|field| field.position), Some(1));
}

fn segment_ids(nodes: &'static [Node], out: &mut Vec<&'static str>) {
    for node in nodes {
        match node {
            Node::Segment(segment) => out.push(segment.segment.id),
            Node::Group(group) => segment_ids(group.children, out),
            Node::Placeholder(_) => {}
        }
    }
}

#[test]
fn emitted_fields_agree_with_the_segment_definitions() {
    let mut fields = 0;
    for segment in &hl7v2_types::segment::SEGMENTS {
        let definition = raw(&format!("{V2_SEGMENT_DIR}/{}.json", segment.id));
        assert_eq!(definition["url"], segment.url);
        let elements = elements(&definition);
        let (root, raw_fields) = elements.split_first().expect("a root element");
        assert_eq!(root["short"], segment.name);
        assert_eq!(raw_fields.len(), segment.fields.len(), "{}", segment.id);
        for (index, (element, field)) in raw_fields.iter().zip(segment.fields).enumerate() {
            fields += 1;
            let context = &field.id;
            assert_eq!(element["id"], field.id);
            assert_eq!(usize::from(field.position), index + 1, "{context}");
            assert!(
                field
                    .id
                    .starts_with(&format!("{}.{}-", segment.id, field.position))
            );
            assert_eq!(element["short"], field.name, "{context}");
            assert_eq!(
                element["type"][0]["code"].as_str(),
                field.data_type,
                "{context}"
            );
            assert_eq!(raw_cardinality(element), field.cardinality, "{context}");
            let optionality =
                extension(element, "http://hl7.org/v2/StructureDefinition/optionality")
                    .expect("optionality");
            assert_eq!(
                optionality["valueCode"].as_str(),
                Some(optionality_code(field.optionality).as_str()),
                "{context}"
            );
            let length = extension(element, "http://hl7.org/v2/StructureDefinition/length");
            assert_eq!(length.is_some(), field.length.is_some(), "{context}");
            if let (Some(length), Some(emitted)) = (length, field.length) {
                assert_eq!(
                    nested(length, "min").and_then(raw_integer),
                    Some(u64::from(emitted.min))
                );
                assert_eq!(
                    nested(length, "max").and_then(raw_integer),
                    emitted.max.map(u64::from)
                );
            }
            let conformance = extension(
                element,
                "http://hl7.org/v2/StructureDefinition/conformance-length",
            );
            assert_eq!(
                conformance.is_some(),
                field.conformance_length.is_some(),
                "{context}"
            );
            if let (Some(conformance), Some(emitted)) = (conformance, field.conformance_length) {
                assert_eq!(
                    nested(conformance, "length").and_then(raw_integer),
                    emitted.length.map(u64::from),
                    "{context}"
                );
                assert_eq!(
                    nested(conformance, "noTruncate").and_then(raw_integer),
                    emitted.no_truncate.map(u64::from),
                    "{context}"
                );
            }
            let value_set = element["binding"]["valueSet"].as_str();
            assert_eq!(
                value_set,
                field.table.map(|table| table.value_set),
                "{context}"
            );
            if let Some(table) = field.table {
                assert_eq!(
                    value_set,
                    Some(format!("http://terminology.hl7.org/ValueSet/v2-{}", table.id).as_str())
                );
            }
            let status = extension(
                element,
                "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
            )
            .and_then(|status| status["valueCode"].as_str());
            let emitted = field.standards_status.map(|status| match status {
                StandardsStatus::Deprecated => "deprecated",
                StandardsStatus::Withdrawn => "withdrawn",
            });
            assert_eq!(status, emitted, "{context}");
        }
    }
    assert_eq!(fields, MODEL.field_count());
    assert_eq!(fields, 2912);
}

/// One structure element as the agreement test compares it.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    cardinality: Cardinality,
    type_code: String,
    status: Option<&'static str>,
}

fn flatten(nodes: &[Node], out: &mut Vec<Entry>) {
    for node in nodes {
        match node {
            Node::Segment(segment) => out.push(Entry {
                id: segment.id.to_owned(),
                cardinality: segment.cardinality,
                type_code: segment.segment.url.to_owned(),
                status: segment.status.map(|status| match status {
                    SegmentStatus::A => "A",
                    SegmentStatus::B => "B",
                    SegmentStatus::D => "D",
                }),
            }),
            Node::Group(group) => {
                out.push(Entry {
                    id: group.id.to_owned(),
                    cardinality: group.cardinality,
                    type_code: String::from("BackboneElement"),
                    status: None,
                });
                let choice = group
                    .children
                    .iter()
                    .all(|child| last_step(node_id(child)).starts_with("choice-"));
                assert_eq!(group.kind == GroupKind::Choice, choice, "{}", group.id);
                flatten(group.children, out);
            }
            Node::Placeholder(slot) => out.push(Entry {
                id: slot.id.to_owned(),
                cardinality: slot.cardinality,
                type_code: String::from("BackboneElement"),
                status: None,
            }),
        }
    }
}

fn node_id(node: &Node) -> &str {
    match node {
        Node::Segment(segment) => segment.id,
        Node::Group(group) => group.id,
        Node::Placeholder(slot) => slot.id,
    }
}

fn last_step(id: &str) -> &str {
    id.rsplit_once('.').map_or(id, |(_, last)| last)
}

#[test]
fn emitted_trees_agree_with_the_structure_definitions() {
    for structure in &hl7v2_types::structure::STRUCTURES {
        let definition = raw(&format!("{V2_STRUCTURE_DIR}/{}.json", structure.id));
        let expected: Vec<Entry> = elements(&definition)
            .iter()
            .skip(1)
            .filter(|element| {
                let id = element["id"].as_str().expect("id");
                !matches!(last_step(id), "segment" | "group")
            })
            .map(|element| Entry {
                id: element["id"].as_str().expect("id").to_owned(),
                cardinality: raw_cardinality(element),
                type_code: element["type"][0]["code"]
                    .as_str()
                    .expect("a type")
                    .to_owned(),
                status: extension(
                    element,
                    "http://hl7.org/v2/StructureDefinition/v2-segment-status",
                )
                .map(|status| status["valueCode"].as_str().expect("a code"))
                .and_then(|code| match code {
                    "A" => Some("A"),
                    "B" => Some("B"),
                    "D" | "d" => Some("D"),
                    "" => None,
                    other => panic!("unexpected segment status {other:?}"),
                }),
            })
            .collect();
        let mut emitted = Vec::new();
        flatten(structure.nodes, &mut emitted);
        assert_eq!(emitted, expected, "{}", structure.id);
    }
}

fn render_tree(nodes: &[Node], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    for node in nodes {
        let cardinality = |count: Cardinality| match count.max {
            Max::Bounded(max) => format!("{}..{max}", count.min),
            Max::Unbounded => format!("{}..*", count.min),
        };
        match node {
            Node::Segment(segment) => {
                writeln!(
                    out,
                    "{indent}{} {} {} {:?}",
                    segment.position,
                    segment.segment.id,
                    cardinality(segment.cardinality),
                    segment.status
                )
                .expect("write");
            }
            Node::Group(group) => {
                let name = group.name;
                writeln!(
                    out,
                    "{indent}{} {name} {} {:?}",
                    group.position,
                    cardinality(group.cardinality),
                    group.kind
                )
                .expect("write");
                render_tree(group.children, depth + 1, out);
            }
            Node::Placeholder(slot) => {
                writeln!(
                    out,
                    "{indent}{} Hxx {}",
                    slot.position,
                    cardinality(slot.cardinality)
                )
                .expect("write");
            }
        }
    }
}

#[test]
fn the_oru_r01_group_tree() {
    let structure = hl7v2_types::structure::find("ORU_R01-A").expect("ORU_R01-A");
    let mut tree = String::new();
    render_tree(structure.nodes, 0, &mut tree);
    insta::assert_snapshot!(tree);
}

#[test]
fn a_placeholder_and_a_choice_are_their_own_nodes() {
    let structure = hl7v2_types::structure::find("QBP_Q15").expect("QBP_Q15");
    assert!(structure.nodes.iter().any(|node| matches!(
        node,
        Node::Placeholder(slot) if slot.id == "QBP_Q15.5-Hxx"
    )));
    let structure = hl7v2_types::structure::find("CCR_I16").expect("CCR_I16");
    let mut entries = Vec::new();
    flatten(structure.nodes, &mut entries);
    assert!(
        entries
            .iter()
            .any(|entry| entry.id.ends_with(".1-CLINICAL_ORDER_OBJECT.choice-3-ODS"))
    );
}

#[test]
fn the_field_table_is_reached_from_the_node() {
    let structure = hl7v2_types::structure::find("ORU_R01-A").expect("ORU_R01-A");
    let Some(Node::Segment(msh)) = structure.nodes.first() else {
        panic!("ORU_R01-A opens with MSH");
    };
    let msh10 = msh.segment.fields.get(9).expect("MSH-10");
    assert_eq!(msh10.id, "MSH.10-messageControlId");
    let obx = hl7v2_types::segment::find("OBX").expect("OBX");
    let obx2 = obx.fields.get(1).expect("OBX-2");
    assert_eq!(obx2.table.map(|table| table.id), Some("0125"));
    assert_eq!(obx2.optionality, Optionality::C);
}

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

#[test]
fn definition_ids_render_as_module_and_static_names() {
    assert_eq!(
        names("ORU_R01-A"),
        (String::from("oru_r01_a"), String::from("ORU_R01_A"))
    );
    assert_eq!(
        names("ACK-Scheduling"),
        (
            String::from("ack_scheduling"),
            String::from("ACK_SCHEDULING")
        )
    );
    assert_eq!(
        names("MFN_Znn"),
        (String::from("mfn_znn"), String::from("MFN_ZNN"))
    );
    assert_eq!(names("OBX"), (String::from("obx"), String::from("OBX")));
    // Windows reserves CON as a file name, so the CON segment's module takes a trailing `_`.
    assert_eq!(names("CON"), (String::from("con_"), String::from("CON")));
    assert_eq!(names("LPT1"), (String::from("lpt1_"), String::from("LPT1")));
    assert_eq!(names("COM"), (String::from("com"), String::from("COM")));
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
    let mut dirs = vec![V2_SEGMENT_DIR, V2_STRUCTURE_DIR];
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

fn tree(root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).expect("dir listable") {
        let path = entry.expect("entry readable").path();
        if path.is_dir() {
            files.extend(tree(&path));
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .into_owned();
            files.push((relative, fs::read_to_string(&path).expect("file readable")));
        }
    }
    files.sort();
    files
}

fn options(crate_dir: &Path, check: bool) -> EmitOptions {
    EmitOptions {
        definitions: definitions_dir(),
        crate_dir: crate_dir.to_path_buf(),
        check,
    }
}

#[test]
fn emitting_twice_is_byte_identical_and_check_passes() {
    let first = tempfile::tempdir().expect("tempdir");
    let second = tempfile::tempdir().expect("tempdir");
    let report = emit(&options(first.path(), false)).expect("first emit");
    emit(&options(second.path(), false)).expect("second emit");
    // lib.rs, model.rs, two index modules, one module per structure and per segment.
    assert_eq!(report.files.len(), 4 + 305 + 190);
    assert_eq!(
        tree(&first.path().join("src")),
        tree(&second.path().join("src"))
    );
    emit(&options(first.path(), true)).expect("check passes on a fresh tree");
    for (path, content) in tree(&first.path().join("src")) {
        assert!(
            content.starts_with("// @generated by fhir-codegen from HL7/v2ig "),
            "{path} starts with the banner"
        );
        assert!(content.contains(
            "DO NOT EDIT.\n// Change the emitter (tools/fhir-codegen) and regenerate.\n// SPDX-FileCopyrightText: Vernum Projecten B.V.\n// SPDX-License-Identifier: Apache-2.0\n"
        ), "{path} carries the SPDX tags under the banner");
    }
}

#[test]
fn check_reports_edited_missing_and_stale_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    emit(&options(dir.path(), false)).expect("emit");
    let src = dir.path().join("src");
    fs::write(src.join("segment/obx.rs"), "// edited\n").expect("edit");
    fs::remove_file(src.join("structure/oru_r01_a.rs")).expect("remove");
    fs::write(src.join("structure/stale.rs"), "// stale\n").expect("stale");
    match emit(&options(dir.path(), true)) {
        Err(EmitError::Tree(fhir_codegen::emit::EmitError::Drift { paths })) => assert_eq!(
            paths,
            vec![
                "segment/obx.rs",
                "structure/oru_r01_a.rs (missing)",
                "structure/stale.rs (stale)"
            ]
        ),
        other => panic!("expected Drift, got {other:?}"),
    }
    emit(&options(dir.path(), false)).expect("a replace removes the stale file");
    emit(&options(dir.path(), true)).expect("and the tree is current again");
}

#[test]
fn the_committed_crate_is_in_sync() {
    emit(&options(&hl7v2_crate_dir(), true))
        .expect("the committed hl7v2-types crate matches the emitter");
}
