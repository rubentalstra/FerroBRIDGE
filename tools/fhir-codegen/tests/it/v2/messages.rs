// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The message definitions and the structures they name.

use fhir_codegen::roots::V2_MESSAGE_DIR;
use hl7v2_types::model::{
    Cardinality, ConformanceLength, DataTypeRef, Field, Group, Length, Max, MessageStatus, Node,
    Optionality, SegmentRef, SegmentStatus, Table,
};

use crate::v2::elements;
use crate::v2::json_files;
use crate::v2::raw;

#[test]
fn every_message_file_is_emitted_with_the_structure_it_names() {
    let files = json_files(V2_MESSAGE_DIR);
    assert_eq!(files.len(), 696);
    assert_eq!(hl7v2_types::message::MESSAGES.len(), files.len());
    for file in &files {
        let definition = raw(file);
        let id = definition["id"].as_str().expect("id");
        let pattern = |element: &str| {
            elements(&definition)
                .iter()
                .find(|entry| entry["id"] == element)
                .map(|entry| entry["patternCode"].as_str().expect("a code").to_owned())
        };
        let code = pattern("Message.messageType").expect("a message type");
        let event = pattern("Message.triggerEvent").unwrap_or_else(|| {
            id.strip_prefix(&format!("{code}-"))
                .expect("the id names the event")
                .to_owned()
        });
        let emitted = hl7v2_types::message::find(&code, &event).expect("every message is emitted");
        assert_eq!(emitted.id, id);
        assert_eq!(definition["url"], emitted.url);
        let status = match pattern("Message.status").as_deref() {
            Some("active") => MessageStatus::Active,
            Some("withdrawn") => MessageStatus::Withdrawn,
            other => panic!("unexpected status {other:?}"),
        };
        assert_eq!(emitted.status, status, "{id}");
        let profile = elements(&definition)
            .iter()
            .find(|entry| entry["id"] == "Message.structure")
            .map(|entry| {
                entry["type"][0]["targetProfile"][0]
                    .as_str()
                    .expect("a profile")
                    .to_owned()
            });
        match (profile, emitted.structure) {
            (Some(profile), Some(structure)) => assert_eq!(
                profile.rsplit_once('/').map(|(_, name)| name.to_owned()),
                Some(structure.id.replace('_', "-")),
                "{id}"
            ),
            (None, None) => {}
            (profile, structure) => panic!("{id}: {profile:?} against {structure:?}"),
        }
    }
    let order: Vec<(&str, &str)> = hl7v2_types::message::MESSAGES
        .iter()
        .map(|message| (message.code, message.event))
        .collect();
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the index is ordered by code and event, with no pair twice"
    );
}

#[test]
fn the_trigger_event_selects_the_structure_variant() {
    let structure = |code: &str, event: &str| {
        hl7v2_types::message::find(code, event)
            .and_then(|message| message.structure)
            .map(|structure| structure.id)
    };
    assert_eq!(structure("ORU", "R01"), Some("ORU_R01-A"));
    assert_eq!(structure("ORU", "R40"), Some("ORU_R01-B"));
    assert_eq!(structure("ADT", "A01"), Some("ADT_A01-A"));
    assert_eq!(structure("ADT", "A04"), Some("ADT_A01-B"));
    assert_eq!(structure("MDM", "T02"), Some("MDM_T02-A"));
    // ACK-A01.json states no trigger event; its id names A01.
    assert_eq!(structure("ACK", "A01"), Some("ACK"));
    let withdrawn = hl7v2_types::message::find("ADT", "A18").expect("ADT^A18");
    assert_eq!(withdrawn.structure, None);
    assert_eq!(withdrawn.status, MessageStatus::Withdrawn);
    assert!(hl7v2_types::message::find("ORU", "Z99").is_none());
}

#[test]
fn the_placeholder_group_name_is_emitted_as_written() {
    let structure = hl7v2_types::structure::find("MDM_T02-A").expect("MDM_T02-A");
    let group = structure
        .nodes
        .iter()
        .find_map(|node| match node {
            Node::Group(group) if group.id == "MDM_T02-A.15-FIXME" => Some(group),
            _ => None,
        })
        .expect("the group at position 15");
    assert_eq!(group.name, "FIXME");
    assert_eq!(group.position, 15);
}

#[test]
fn the_tree_and_table_types_compare_by_value() {
    let structure = hl7v2_types::structure::find("ORU_R01-A").expect("ORU_R01-A");
    assert_eq!(
        structure.nodes.first(),
        Some(&Node::Segment(SegmentRef {
            id: "ORU_R01-A.1-MSH",
            position: 1,
            segment: &hl7v2_types::segment::msh::MSH,
            cardinality: Cardinality {
                min: 1,
                max: Max::Bounded(1),
            },
            status: Some(SegmentStatus::A),
        }))
    );
    assert!(matches!(
        structure.nodes.get(4),
        Some(Node::Group(Group {
            name: "PATIENT_RESULT",
            ..
        }))
    ));
    let obx = hl7v2_types::segment::find("OBX").expect("OBX");
    assert_eq!(
        obx.fields.get(1),
        Some(&Field {
            id: "OBX.2-valueType",
            position: 2,
            name: "Value Type",
            data_type: Some(DataTypeRef::Defined(&hl7v2_types::data_type::id::ID)),
            cardinality: Cardinality {
                min: 0,
                max: Max::Bounded(1),
            },
            optionality: Optionality::C,
            length: Some(Length {
                min: 2,
                max: Some(3),
            }),
            conformance_length: None,
            table: Some(Table {
                id: "0125",
                value_set: "http://terminology.hl7.org/ValueSet/v2-0125",
            }),
            standards_status: None,
        })
    );
    let cx = hl7v2_types::data_type::find("CX").expect("CX");
    let first = cx.components.first().expect("CX.1");
    assert_eq!(first.data_type, Some(&hl7v2_types::data_type::st::ST));
    assert_eq!(
        first.conformance_length,
        Some(ConformanceLength {
            length: Some(15),
            no_truncate: Some(true),
        })
    );
    assert_ne!(
        hl7v2_types::structure::find("ORU_R01-A"),
        hl7v2_types::structure::find("ORU_R01-B")
    );
}
