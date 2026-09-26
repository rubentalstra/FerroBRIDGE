// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every structure and segment file emitted, and the field tables against the
//! segment definitions.

use fhir_codegen::roots::{V2_SEGMENT_DIR, V2_STRUCTURE_DIR};
use hl7v2_types::model::{DataTypeRef, Node, StandardsStatus};

use crate::v2::MODEL;
use crate::v2::data_types::assert_lengths;
use crate::v2::elements;
use crate::v2::extension;
use crate::v2::json_files;
use crate::v2::optionality_code;
use crate::v2::raw;
use crate::v2::raw_cardinality;

#[test]
fn every_structure_file_and_every_segment_file_is_emitted() {
    let files = json_files(V2_STRUCTURE_DIR);
    assert_eq!(files.len(), 305);
    assert_eq!(hl7v2_types::structure::STRUCTURES.len(), files.len());
    for file in &files {
        let id = raw(file)["id"].as_str().expect("id").to_owned();
        let emitted = hl7v2_types::structure::find(&id).expect("every structure is emitted");
        assert_eq!(
            emitted.url.map(str::to_owned),
            Some(format!("http://hl7.org/v2/StructureDefinition/{id}"))
        );
        assert_eq!(emitted.version, "2.9.1");
        assert_eq!(emitted.withdrawn_as_of, None);
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
        assert_eq!(definition["url"].as_str(), segment.url);
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
                field.data_type.map(|data_type| data_type.code()),
                "{context}"
            );
            match field.data_type {
                Some(DataTypeRef::Defined(data_type)) => assert_eq!(
                    hl7v2_types::data_type::find(data_type.code),
                    Some(data_type),
                    "{context}"
                ),
                Some(DataTypeRef::Undefined(code)) => {
                    assert!(hl7v2_types::data_type::find(code).is_none(), "{context}");
                }
                Some(DataTypeRef::Legacy(data_type)) => {
                    panic!(
                        "{context}: a v2.9.1 field carries the legacy code {}",
                        data_type.code
                    )
                }
                None => {}
            }
            assert_eq!(raw_cardinality(element), field.cardinality, "{context}");
            let optionality =
                extension(element, "http://hl7.org/v2/StructureDefinition/optionality")
                    .expect("optionality");
            assert_eq!(
                optionality["valueCode"].as_str(),
                Some(optionality_code(field.optionality).as_str()),
                "{context}"
            );
            assert_lengths(element, field.length, field.conformance_length, context);
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
