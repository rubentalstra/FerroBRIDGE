// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The group trees against the structure definitions, and the names a
//! definition id renders as.

use std::fmt::Write;

use fhir_codegen::roots::V2_STRUCTURE_DIR;
use fhir_codegen::v2::render::names;
use hl7v2_types::model::{Cardinality, GroupKind, Max, Node, Optionality, SegmentStatus};

use crate::v2::elements;
use crate::v2::extension;
use crate::v2::raw;
use crate::v2::raw_cardinality;

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
                type_code: segment
                    .segment
                    .url
                    .expect("a v2.9.1 segment has a URL")
                    .to_owned(),
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
    // `fn` is a Rust keyword, so the FN data type's module takes a trailing `_`.
    assert_eq!(names("FN"), (String::from("fn_"), String::from("FN")));
    assert_eq!(
        names("ACK-O59_A"),
        (String::from("ack_o59_a"), String::from("ACK_O59_A"))
    );
}
