// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

use openehr_rm::v1_2::paths::RmPath;

use super::matching::archetype_release_version;
use super::matching::interface_form;
use super::matching::matches_node;
use super::matching::node_id_matches;
use super::matching::parse_aql_path;

fn path(rendered: &str) -> RmPath {
    parse_aql_path(rendered).expect("a well-formed path")
}

#[test]
fn only_a_full_archetype_id_carries_a_release_version() {
    assert_eq!(
        archetype_release_version("openEHR-EHR-EVALUATION.note.v1.4.1"),
        Some("1.4.1")
    );
    assert_eq!(
        archetype_release_version("org.example::openEHR-EHR-ACTION.consent.v0.0.1-alpha"),
        Some("0.0.1-alpha")
    );
    assert_eq!(
        archetype_release_version("openEHR-EHR-EVALUATION.note.v1"),
        None
    );
    assert_eq!(archetype_release_version("at0001"), None);
}

#[test]
fn an_archetype_id_matches_in_its_interface_form() {
    assert!(node_id_matches(
        "openEHR-EHR-EVALUATION.note.v1.0.0",
        "openEHR-EHR-EVALUATION.note.v1"
    ));
    assert!(node_id_matches(
        "org.example::openEHR-EHR-EVALUATION.note.v1.0.0",
        "openEHR-EHR-EVALUATION.note.v1"
    ));
    assert!(!node_id_matches(
        "openEHR-EHR-EVALUATION.note.v2",
        "openEHR-EHR-EVALUATION.note.v1"
    ));
    assert_eq!(interface_form("at0001"), "at0001");
}

#[test]
fn a_positional_predicate_selects_an_instance_and_not_a_node() {
    // BASE master11-paths, Using Positional Parameters: the position stands
    // alone or joins the node id as a conjunct; it never changes which node
    // a path names, only which instance of it.
    let node = path("/content[openEHR-EHR-EVALUATION.note.v1]/data[at0001]/items[at0002]");
    let alone = path("/content[openEHR-EHR-EVALUATION.note.v1]/data[at0001]/items[2]");
    let joined = path("/content[openEHR-EHR-EVALUATION.note.v1]/data[at0001]/items[at0002 and 2]");
    assert!(matches_node(&alone, &node, None));
    assert!(matches_node(&joined, &node, None));
    assert_eq!(
        joined.segments.last().and_then(|s| s.predicate.position),
        Some(2)
    );
}

#[test]
fn a_name_predicate_refuses_a_node_with_another_fixed_name() {
    let node = path("/data[at0001]/items[at0002]");
    let query = path("/data[at0001]/items[at0002 and name/value='systolic']");
    assert!(matches_node(&query, &node, Some("systolic")));
    assert!(!matches_node(&query, &node, Some("diastolic")));
    assert!(matches_node(&query, &node, None));
}

#[test]
fn a_shorter_or_longer_path_is_not_the_node() {
    let node = path("/data[at0001]/items[at0002]");
    assert!(!matches_node(&path("/data[at0001]"), &node, None));
    assert!(!matches_node(
        &path("/data[at0001]/items[at0002]/value"),
        &node,
        None
    ));
}
