// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `aqlPath` index over an ADL 1.4 template, and the composition seam.
//!
//! The fixture is the synthetic operational template of the testkit: one
//! `EVALUATION` with one `DV_TEXT` element under an `ITEM_TREE`. Every
//! expectation below is read from the Web Template the openEHR builder emits,
//! never from a hand-written list of paths.

use core::str::FromStr;

use ferrobridge_testkit::fixtures::MINIMAL_EVALUATION_NOTE;
use ferrobridge_testkit::fixtures::MINIMAL_EVALUATION_OPT;
use ferrobridge_testkit::fixtures::MINIMAL_EVALUATION_TEMPLATE_ID;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::paths::AqlPath;
use openehr_mapping_core::index::paths::relative;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::template::Generation;
use openehr_mapping_core::template::PathError;
use openehr_mapping_core::template::TemplateSource;
use openehr_rm::v1_2::paths::RmPath;
use proptest::prelude::ProptestConfig;
use proptest::prop_assert;
use proptest::proptest;
use proptest::sample::select;

/// The `aqlPath` of the one `DV_TEXT` leaf of the fixture.
const NOTE_TEXT: &str =
    "/content[openEHR-EHR-EVALUATION.ferrobridge_note.v1]/data[at0001]/items[at0002]/value";

/// The timestamp the builder fills the `ctx/time` default from.
const NOW: &str = "2026-09-13T08:00:00Z";

/// Builds the index over the testkit's ADL 1.4 fixture.
#[expect(clippy::expect_used, reason = "test plumbing")]
fn fixture_index() -> WebTemplateIndex {
    let opt = openehr_its::opt14::from_xml(MINIMAL_EVALUATION_OPT).expect("the fixture parses");
    WebTemplateIndex::build(&TemplateSource::Opt14(Box::new(opt))).expect("the fixture indexes")
}

/// Returns the anchor a path with no head variable resolves against.
fn root_anchor() -> RmPath {
    RmPath {
        absolute: true,
        segments: Vec::new(),
    }
}

/// Returns the values the composition of the fixture is built from.
#[expect(clippy::expect_used, reason = "test plumbing")]
fn fixture_values(index: &WebTemplateIndex) -> Vec<NodeValue> {
    let language = index
        .node(&AqlPath::new("/language"))
        .expect("the composition language node");
    let territory = index
        .node(&AqlPath::new("/territory"))
        .expect("the composition territory node");
    let composer = index
        .node(&AqlPath::new("/composer"))
        .expect("the composition composer node");
    let note = index.node(&AqlPath::new(NOTE_TEXT)).expect("the note node");
    vec![
        NodeValue::new(language, "en".into()).with_datum("code"),
        NodeValue::new(territory, "NL".into()).with_datum("code"),
        NodeValue::new(composer, "FerroBRIDGE".into()).with_datum("name"),
        NodeValue::new(note, MINIMAL_EVALUATION_NOTE.into()),
    ]
}

#[test]
fn the_template_builds_and_names_its_generation() {
    let index = fixture_index();
    assert_eq!(index.template_id(), MINIMAL_EVALUATION_TEMPLATE_ID);
    assert_eq!(index.generation(), Generation::Adl14);
    assert_eq!(index.root().rm_type(), "COMPOSITION");
    assert_eq!(
        index.root().node_id(),
        Some("openEHR-EHR-COMPOSITION.ferrobridge_minimal.v1")
    );
}

#[test]
fn every_aql_path_resolves_to_exactly_one_node() {
    let index = fixture_index();
    let paths: Vec<AqlPath> = index
        .nodes()
        .map(|node| node.aql_path().clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(paths.len(), index.nodes().count(), "{paths:?}");
    for path in &paths {
        let node = index.node(path).expect("every indexed path resolves");
        assert_eq!(node.aql_path(), path);
    }
}

#[test]
fn a_positional_predicate_in_a_mapping_path_is_refused_not_dropped() {
    // BASE master11-paths, Using Positional Parameters: a position selects an
    // instance, and instances travel as structured occurrences into a build or
    // a read; a position written inside a path would be dropped silently.
    let index = fixture_index();
    let positioned = "/content[openEHR-EHR-EVALUATION.ferrobridge_note.v1]/data[at0001]/items[at0002 and 2]/value";
    let path = MappingPath::from_str(positioned).expect("a well-formed mapping path");
    let error = index
        .resolve(&path, &root_anchor())
        .expect_err("a positional predicate is refused");
    assert!(
        matches!(error, PathError::PositionalPredicate { ref path } if path.contains("/items[2]/value")),
        "expected a positional-predicate refusal, got {error:?}"
    );
}

#[test]
fn a_resolved_leaf_carries_its_rm_type_and_occurrence_bounds() {
    let index = fixture_index();
    let path = MappingPath::from_str(NOTE_TEXT).expect("a well-formed mapping path");
    let node = index
        .resolve(&path, &root_anchor())
        .expect("the leaf resolves");
    assert_eq!(node.rm_type(), "DV_TEXT");
    assert_eq!(node.node_id(), Some("at0002"));
    assert_eq!(node.min(), Some(1));
    assert_eq!(node.max(), Some(1));
    assert!(!node.repeats());
    assert_eq!(
        node.flat_id().as_str(),
        "minimal_evaluation/synthetic_note/note_text"
    );
}

#[test]
fn an_archetype_id_predicate_matches_the_interface_form() {
    let index = fixture_index();
    let path = MappingPath::from_str(
        "/content[org.example::openEHR-EHR-EVALUATION.ferrobridge_note.v1.2.3]",
    )
    .expect("a well-formed mapping path");
    let node = index
        .resolve(&path, &root_anchor())
        .expect("the archetype root resolves");
    assert_eq!(node.rm_type(), "EVALUATION");
}

#[test]
fn a_path_outside_the_template_names_its_nearest_ancestor() {
    let index = fixture_index();
    let path = MappingPath::from_str(
        "/content[openEHR-EHR-EVALUATION.ferrobridge_note.v1]/data[at0001]/items[at0099]/value",
    )
    .expect("a well-formed mapping path");
    let error = index
        .resolve(&path, &root_anchor())
        .expect_err("at0099 is not in the template");
    match error {
        PathError::UnknownPath {
            ref nearest_ancestor,
            ..
        } => assert_eq!(
            nearest_ancestor.as_deref(),
            Some("/content[openEHR-EHR-EVALUATION.ferrobridge_note.v1]")
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_parent_step_resolves_against_its_anchor_before_the_match() {
    let index = fixture_index();
    let anchor = RmPath::from_str(
        "/content[openEHR-EHR-EVALUATION.ferrobridge_note.v1]/data[at0001]/items[at0002]/value",
    )
    .expect("a well-formed anchor");
    let path = MappingPath::from_str("../../..").expect("a well-formed mapping path");
    let node = index.resolve(&path, &anchor).expect("the entry resolves");
    assert_eq!(node.rm_type(), "EVALUATION");
}

#[test]
fn a_composition_builds_validates_and_reads_back() {
    let index = fixture_index();
    let composition = index
        .build_composition(&fixture_values(&index), NOW)
        .expect("the values build a valid composition");
    assert_eq!(composition.template_id(), MINIMAL_EVALUATION_TEMPLATE_ID);
    assert_eq!(composition.generation(), Generation::Adl14);
    let note = index.node(&AqlPath::new(NOTE_TEXT)).expect("the note node");
    let read = index
        .read(&composition, note, &[])
        .expect("the note reads back")
        .expect("the note is present");
    assert_eq!(
        read.get("value").and_then(|v| v.as_str()),
        Some(MINIMAL_EVALUATION_NOTE)
    );
    insta::assert_json_snapshot!("minimal_evaluation_composition", composition.value());
}

#[test]
fn an_absent_optional_node_reads_as_none() {
    let index = fixture_index();
    let composition = index
        .build_composition(&fixture_values(&index), NOW)
        .expect("the values build a valid composition");
    let setting = index
        .node(&AqlPath::new("/context/setting"))
        .expect("the setting node");
    assert_eq!(
        index
            .read(&composition, setting, &[])
            .expect("an absent node is not an error"),
        None
    );
}

#[test]
fn an_occurrence_list_that_does_not_match_the_template_is_refused() {
    let index = fixture_index();
    let note = index.node(&AqlPath::new(NOTE_TEXT)).expect("the note node");
    let error = index
        .read(&composition_of(&index), note, &[RmPosition::first()])
        .expect_err("the leaf has no repeating node above it");
    assert!(
        matches!(
            error,
            PathError::OccurrenceCount {
                expected: 0,
                given: 1,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_context_value_and_a_raw_value_build_the_same_composition() {
    // Simplified Formats master06 §Composer, §Language and Territory: the
    // `ctx/` keys set what the path spellings set; master04 §Raw canonical
    // JSON: a leaf takes its whole canonical value under `|raw`.
    let index = fixture_index();
    let note = index.node(&AqlPath::new(NOTE_TEXT)).expect("the note node");
    let values = vec![
        NodeValue::context("language", "en".into()),
        NodeValue::context("territory", "NL".into()),
        NodeValue::context("composer_name", "FerroBRIDGE".into()),
        NodeValue::new(
            note,
            serde_json::json!({"_type": "DV_TEXT", "value": MINIMAL_EVALUATION_NOTE}),
        )
        .with_datum("raw"),
    ];
    assert_eq!(values.first().and_then(NodeValue::flat_id), None);
    let built = index
        .build_composition(&values, NOW)
        .expect("the context and raw values build");
    assert_eq!(built.value(), composition_of(&index).value());
}

#[test]
fn an_instance_index_past_the_flat_bound_is_refused() {
    // openehr_sdt::flat::path::MAX_INSTANCE_INDEX is the largest `:i` the FLAT
    // reader accepts, so a key past it is refused before the builder sees it.
    let index = fixture_index();
    let note = index.node(&AqlPath::new(NOTE_TEXT)).expect("the note node");
    let past = openehr_sdt::flat::path::MAX_INSTANCE_INDEX + 1;
    let values = vec![
        NodeValue::new(note, MINIMAL_EVALUATION_NOTE.into()).under(vec![
            openehr_sdt::flat::path::Segment {
                name: String::from("_mapping"),
                index: Some(past),
            },
        ]),
    ];
    let error = index
        .build_composition(&values, NOW)
        .expect_err("the index is past the bound");
    assert!(
        matches!(error, PathError::InstanceIndex { index, .. } if index == past),
        "{error:?}"
    );
}

/// Builds the fixture's composition.
#[expect(clippy::expect_used, reason = "test plumbing")]
fn composition_of(
    index: &WebTemplateIndex,
) -> openehr_mapping_core::composition::CanonicalComposition {
    index
        .build_composition(&fixture_values(index), NOW)
        .expect("the values build a valid composition")
}

#[test]
fn a_composition_of_another_template_is_refused() {
    let index = fixture_index();
    let foreign = openehr_mapping_core::composition::CanonicalComposition::new(
        serde_json::Value::Null,
        "other.template.v1",
        Generation::Adl14,
    );
    let error = index
        .read(&foreign, index.root(), &[])
        .expect_err("the composition names another template");
    assert!(
        matches!(error, PathError::TemplateMismatch { .. }),
        "{error:?}"
    );
}

#[test]
fn a_relative_path_is_the_child_path_without_the_parent_prefix() {
    let index = fixture_index();
    let entry = index
        .node(&AqlPath::new(
            "/content[openEHR-EHR-EVALUATION.ferrobridge_note.v1]",
        ))
        .expect("the entry node");
    let note = index.node(&AqlPath::new(NOTE_TEXT)).expect("the note node");
    let path = relative(entry, note).expect("the note is under the entry");
    assert_eq!(path.to_string(), "data[at0001]/items[at0002]/value");
    assert!(
        relative(note, note)
            .expect("a node is under itself")
            .is_empty()
    );
    assert!(relative(note, entry).is_err());
}

/// Returns every node of the fixture, for the property below.
fn fixture_nodes() -> Vec<ResolvedNode> {
    fixture_index().nodes().cloned().collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Derivation is total over the node set: every pair either yields a
    /// relative path that reproduces the child's `aqlPath`, or is refused as
    /// not a descendant.
    #[test]
    fn relative_derivation_is_total_over_the_node_set(
        (parent, child) in (select(fixture_nodes()), select(fixture_nodes()))
    ) {
        match relative(&parent, &child) {
            Ok(path) => {
                let rendered = path.to_string();
                let joined = if rendered.is_empty() {
                    parent.aql_path().as_str().to_owned()
                } else {
                    format!("{}/{rendered}", parent.aql_path())
                };
                prop_assert!(
                    joined == child.aql_path().as_str(),
                    "{joined} is not {}",
                    child.aql_path()
                );
            }
            Err(error) => prop_assert!(
                matches!(error, PathError::NotADescendant { .. }),
                "{error:?}"
            ),
        }
    }
}
