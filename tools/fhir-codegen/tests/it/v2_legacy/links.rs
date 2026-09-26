// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The message index and the statics a legacy tree or segment links to.

use fhir_codegen::v2::legacy::lower::Owner;
use fhir_codegen::v2::legacy::source::version_key;
use hl7v2_types::model::{DataTypeRef, Node, Structure};

use crate::v2_legacy::LEGACY;

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
pub(super) fn references(
    nodes: &'static [Node],
    out: &mut Vec<&'static hl7v2_types::model::Segment>,
) {
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
    assert_eq!(LEGACY.segment_count(), 720);
    assert_eq!(LEGACY.shared_count(), 446);
    assert_eq!(LEGACY.inherited_count(), 327);
}

/// The emitted static of the structure `id` in the tables of `version`.
fn emitted(id: &str, version: &str) -> &'static Structure {
    hl7v2_types::legacy::find(id, version).expect("the structure is emitted at the version")
}

#[test]
fn a_tree_identical_to_an_earlier_one_shares_its_nodes() {
    let mut linked = 0;
    for version in &LEGACY.versions {
        for (id, owner) in &version.trees {
            let structure = emitted(id, &version.version);
            let nodes = match owner {
                Owner::Current => {
                    hl7v2_types::structure::find(id)
                        .expect("a v2.9.1 structure")
                        .nodes
                }
                Owner::Version(earlier) => emitted(id, earlier).nodes,
            };
            assert!(
                std::ptr::eq(structure.nodes, nodes),
                "{id} {} shares the tree of {owner:?}",
                version.version
            );
            assert_eq!(structure.version, version.version);
            linked += 1;
        }
    }
    assert_eq!(
        linked,
        LEGACY.current_tree_count() + LEGACY.inherited_tree_count()
    );
    assert_eq!(LEGACY.tree_count() + linked, LEGACY.structure_count());
    assert!(LEGACY.inherited_tree_count() > 0);
}

#[test]
fn a_segment_identical_to_an_earlier_version_s_is_linked_to_that_static() {
    let mut inherited = 0;
    for version in &LEGACY.versions {
        for (id, owner) in &version.links {
            let Owner::Version(earlier) = owner else {
                continue;
            };
            let from = LEGACY
                .versions
                .iter()
                .find(|other| other.version == *earlier)
                .expect("the owner is a lowered version");
            assert!(
                from.segments.contains_key(id),
                "{id} is emitted at {earlier}"
            );
            inherited += 1;
        }
    }
    assert_eq!(inherited, LEGACY.inherited_count());
}
