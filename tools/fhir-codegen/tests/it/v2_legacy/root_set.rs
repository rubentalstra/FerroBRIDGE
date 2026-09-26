// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legacy root set, the withdrawals and table 0354.

use std::collections::{BTreeMap, BTreeSet};

use fhir_codegen::v2::legacy::lower::LegacyDefect;
use fhir_codegen::v2::legacy::source::{CodeStatus, TABLE_0354};
use hl7v2_types::model::{Max, Node, Structure};

use crate::v2_legacy::LEGACY;
use crate::v2_legacy::TABLE;
use crate::v2_legacy::TABLES;
use crate::v2_legacy::current_codes;
use crate::v2_legacy::lowered_structures;
use crate::v2_legacy::raw;
use crate::v2_legacy::rooted_structures;

#[test]
fn the_root_set_is_every_rooted_structure_the_v2_9_1_definitions_lack() {
    let current = current_codes();
    let expected: BTreeMap<String, BTreeSet<String>> = rooted_structures()
        .into_iter()
        .filter(|(id, _)| !current.contains(id))
        .collect();
    let lowered: BTreeMap<String, BTreeSet<String>> = lowered_structures()
        .into_iter()
        .filter(|(id, _)| !current.contains(id))
        .collect();
    assert_eq!(lowered, expected);
    assert_eq!(expected.len(), 57);
    assert_eq!(LEGACY.withdrawn_count(), 218);
    assert_eq!(LEGACY.withdrawn_code_count(), 57);
    for structure in &hl7v2_types::legacy::STRUCTURES {
        assert_eq!(structure.url, None);
        assert_eq!(
            structure.withdrawn_as_of.is_some(),
            !current.contains(structure.id),
            "{} is withdrawn exactly when v2.9.1 lacks it",
            structure.id
        );
    }
}

#[test]
fn the_root_set_is_every_rooted_structure_at_every_version_but_the_damaged_trees() {
    // NOTE: the 2.8.1 and 2.8.2 tables hang the donation structures' elements under other
    // roots (LegacyDefect::EmptyRoot, ForeignGroup, SecondHeader), so those trees are left out.
    let rooted = rooted_structures();
    let lowered = lowered_structures();
    let mut left_out: BTreeSet<(String, String)> = BTreeSet::new();
    for (id, versions) in &rooted {
        let kept = lowered.get(id).cloned().unwrap_or_default();
        assert!(kept.is_subset(versions), "{id}: {kept:?} of {versions:?}");
        for version in versions.difference(&kept) {
            left_out.insert((version.clone(), id.clone()));
        }
    }
    assert!(
        left_out
            .iter()
            .all(|(version, _)| version == "2.8.1" || version == "2.8.2"),
        "{left_out:?}"
    );
    let left_out_ids: BTreeSet<&str> = left_out.iter().map(|(_, id)| id.as_str()).collect();
    assert_eq!(
        left_out_ids,
        BTreeSet::from([
            "CSU_C09", "DBC_O41", "DBC_O42", "DEL_O46", "DEO_O45", "DER_O44", "DFT_P11", "DPR_O48",
            "DRC_O47", "DRG_O43", "QBP_E22", "QBP_O33", "QBP_O34", "RSP_K32", "RSP_O33", "RSP_O34",
        ])
    );
    let absent: BTreeSet<&str> = rooted
        .keys()
        .map(String::as_str)
        .filter(|id| !lowered.contains_key(*id))
        .collect();
    assert!(absent.is_subset(&left_out_ids), "{absent:?}");
    assert_eq!(LEGACY.code_count(), rooted.len() - absent.len());
    assert_eq!(
        hl7v2_types::legacy::STRUCTURES.len(),
        LEGACY.structure_count()
    );
    let versions: Vec<&str> = LEGACY.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(
        versions,
        [
            "2.3", "2.3.1", "2.4", "2.5", "2.5.1", "2.6", "2.7", "2.7.1", "2.8", "2.8.1", "2.8.2"
        ]
    );
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
    let current = current_codes();
    for structure in &hl7v2_types::legacy::STRUCTURES {
        if current.contains(structure.id) {
            assert_eq!(structure.withdrawn_as_of, None, "{}", structure.id);
            continue;
        }
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
        .filter(|structure| structure.withdrawn_as_of.is_some())
        .map(|structure| structure.id)
        .collect();
    assert_eq!(codes.len(), 57);
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
        LegacyDefect::UnlistedSecondRoot,
        LegacyDefect::EmptyRoot,
        LegacyDefect::ForeignGroup,
        LegacyDefect::SecondHeader,
        LegacyDefect::DotLength,
        LegacyDefect::MissingField,
        LegacyDefect::StrayRequired,
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
