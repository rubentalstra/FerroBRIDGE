// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The path algebra: `../` resolution against an anchor, over generated
//! anchors and depths.
//!
//! The invariant the two interpreters depend on is that a resolved path is an
//! openEHR path: the parent step is the mapping languages' own operator
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/path_operators.adoc`),
//! and the openEHR path grammar of BASE Release 1.2.0 §Paths and Locators has
//! no `..`, so no `..` may survive resolution.

use core::str::FromStr;

use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::path::PathResolutionError;
use openehr_rm::v1_2::paths::RmPath;
use proptest::prelude::Just;
use proptest::prelude::ProptestConfig;
use proptest::prelude::Strategy;
use proptest::prop_assert;
use proptest::prop_assert_eq;
use proptest::proptest;

/// Builds an absolute anchor of `depth` segments with at-coded predicates.
#[expect(
    clippy::expect_used,
    reason = "the rendered anchor is at-coded and slash-separated by construction"
)]
fn anchor_of_depth(depth: usize) -> RmPath {
    let segments: Vec<String> = (0..depth)
        .map(|index| format!("/items[at{:04}]", index + 1))
        .collect();
    let rendered = segments.concat();
    if rendered.is_empty() {
        return RmPath {
            absolute: true,
            segments: Vec::new(),
        };
    }
    RmPath::from_str(&rendered).expect("a generated anchor should parse")
}

/// Builds a mapping path of `steps` parent steps and `tail` trailing segments.
#[expect(
    clippy::expect_used,
    reason = "the rendered path is a parent-step run over plain attribute names"
)]
fn path_of(steps: usize, tail: usize) -> MappingPath {
    let segments: Vec<String> = (0..tail).map(|index| format!("value{index}")).collect();
    let mut rendered = "../".repeat(steps);
    rendered.push_str(&segments.join("/"));
    if rendered.is_empty() {
        rendered.push_str("value0");
    }
    MappingPath::from_str(&rendered).expect("a generated path should parse")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn resolution_never_yields_a_parent_step(
        (depth, steps, tail) in (0_usize..8).prop_flat_map(|depth| {
            (Just(depth), 0_usize..=depth, 0_usize..4)
        })
    ) {
        let anchor = anchor_of_depth(depth);
        let path = path_of(steps, tail);
        let resolved = path.resolve(&anchor).expect("the anchor is deep enough");
        prop_assert!(!resolved.to_string().contains(".."));
        prop_assert!(
            resolved.segments.iter().all(|segment| segment.attribute != "..")
        );
    }

    #[test]
    fn depth_accounting_is_exact(
        (depth, steps, tail) in (0_usize..8).prop_flat_map(|depth| {
            (Just(depth), 0_usize..=depth, 1_usize..4)
        })
    ) {
        let anchor = anchor_of_depth(depth);
        let path = path_of(steps, tail);
        let resolved = path.resolve(&anchor).expect("the anchor is deep enough");
        prop_assert_eq!(resolved.segments.len(), depth - steps + tail);
        prop_assert_eq!(resolved.absolute, anchor.absolute);
    }

    #[test]
    fn one_step_above_the_anchor_root_is_always_refused(
        (depth, extra) in (0_usize..8).prop_flat_map(|depth| (Just(depth), 1_usize..4))
    ) {
        let anchor = anchor_of_depth(depth);
        let steps = depth + extra;
        let path = path_of(steps, 1);
        prop_assert_eq!(
            path.resolve(&anchor),
            Err(PathResolutionError::AboveAnchorRoot { steps, anchor_depth: depth })
        );
    }

    #[test]
    fn the_anchor_prefix_is_preserved(
        (depth, steps) in (1_usize..8).prop_flat_map(|depth| (Just(depth), 0_usize..depth))
    ) {
        let anchor = anchor_of_depth(depth);
        let path = path_of(steps, 0);
        let resolved = path.resolve(&anchor).expect("the anchor is deep enough");
        let kept = depth - steps;
        prop_assert_eq!(&resolved.segments[..kept], &anchor.segments[..kept]);
    }
}
