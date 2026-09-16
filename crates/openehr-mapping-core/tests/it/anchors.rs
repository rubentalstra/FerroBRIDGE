// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Anchors, aliases and merge keys resolve before a value tree is handed on.
//!
//! 57 of the 202 OMOCL corpus files define an anchor, so a loader that left
//! aliases unresolved would carry an unusable tree into both interpreters.

use std::error::Error;
use std::path::PathBuf;

use openehr_mapping_core::loader;
use openehr_mapping_core::position::Position;
use openehr_mapping_core::value::MappingValue;
use openehr_mapping_core::value::PositionedValue;

/// A vendored corpus file, reached from this crate's manifest directory.
fn corpus_file(relative: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(relative)
}

/// Returns the `mappings` record at `index` of a loaded document.
fn record(document: &PositionedValue, index: usize) -> Option<&PositionedValue> {
    document.get("mappings")?.as_sequence()?.get(index)
}

/// Returns the `path` node of the first entry of an `alternatives` sequence.
fn alternative_path(alternatives: &PositionedValue) -> Option<&PositionedValue> {
    alternatives.as_sequence()?.first()?.get("path")
}

#[test]
fn an_alias_carries_the_anchored_value_and_its_own_position() -> Result<(), Box<dyn Error>> {
    let file = corpus_file("docs/specs/omocl/medical_data/observation/Blood_pressure_v2.yml");
    let document = loader::load_file(file)?;
    let first = record(document.document(), 0).ok_or("no first mappings record")?;

    let anchored = first
        .get("value")
        .and_then(|value| value.get("alternatives"))
        .ok_or("no anchored alternatives")?;
    let aliased = first
        .get("range_low")
        .and_then(|value| value.get("alternatives"))
        .ok_or("no aliased alternatives")?;

    let anchored_path = alternative_path(anchored).ok_or("no anchored path")?;
    let aliased_path = alternative_path(aliased).ok_or("no aliased path")?;
    assert_eq!(
        anchored_path.as_text(),
        aliased_path.as_text(),
        "an alias carries the anchored value"
    );
    assert_eq!(anchored_path.position(), Position::new(22, 17));
    assert_eq!(
        aliased_path.position(),
        Position::new(28, 21),
        "an alias is positioned at its own use, not at the anchor"
    );
    Ok(())
}

#[test]
fn a_merge_key_expands_into_the_surrounding_mapping() -> Result<(), Box<dyn Error>> {
    let source = "defaults: &defaults\n  system: OMOP\n  version: 5.4\nspec:\n  <<: *defaults\n  \
                  openEhrConfig:\n    archetype: openEHR-EHR-CLUSTER.device.v1\n";
    let document = loader::parse_str("merge.yml", source)?;
    let spec = document.get("spec").ok_or("no spec node")?;
    assert_eq!(
        spec.get("system").and_then(PositionedValue::as_text),
        Some("OMOP")
    );
    assert!(matches!(
        spec.get("version").map(PositionedValue::value),
        Some(&MappingValue::Float(_))
    ));
    assert!(
        spec.get("<<").is_none(),
        "the merge key is expanded, never kept as a key"
    );
    Ok(())
}
