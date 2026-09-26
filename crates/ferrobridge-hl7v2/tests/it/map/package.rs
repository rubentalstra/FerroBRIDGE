// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The vendored package as loaded: its maps by kind and its conditions.

use std::collections::BTreeMap;

use ferrobridge_hl7v2::map::corpus::{Condition, Kind};

use crate::support::{self};

#[test]
fn the_package_loads_every_concept_map_of_the_four_kinds() {
    let corpus = support::corpus();
    let count = |kind| corpus.maps().filter(|map| map.kind == kind).count();
    assert_eq!(count(Kind::Message), 13);
    assert_eq!(count(Kind::Segment), 75);
    assert_eq!(count(Kind::Datatype), 105);
    assert_eq!(count(Kind::Table), 70);
    assert_eq!(corpus.maps().count(), 263);
}

#[test]
fn the_corpus_conditions_parse_or_are_refused_as_counted_forms() {
    let corpus = support::corpus();
    let mut computable = 0usize;
    let mut refused = 0usize;
    let mut narrative = 0usize;
    let mut defective = BTreeMap::new();
    let mut unsupported = BTreeMap::new();
    for row in corpus.maps().flat_map(|map| map.rows.iter()) {
        match &row.condition {
            Condition::Computable(_) => computable += 1,
            Condition::Unsupported { text, .. } => {
                refused += 1;
                *unsupported.entry(text.clone()).or_insert(0usize) += 1;
            }
            Condition::Defective { text } => {
                refused += 1;
                *defective.entry(text.clone()).or_insert(0usize) += 1;
            }
            Condition::Narrative => narrative += 1,
            Condition::Always => {}
        }
    }
    let list = |forms: &BTreeMap<String, usize>| {
        forms
            .iter()
            .flat_map(|(text, count)| [format!("  {count} "), text.clone(), String::from("\n")])
            .collect::<String>()
    };
    insta::assert_snapshot!(
        "corpus_conditions",
        format!(
            "computable: {computable}\nrefused: {refused}\nnarrative: {narrative}\n\
             defective (no operand):\n{}unsupported:\n{}",
            list(&defective),
            list(&unsupported)
        )
    );
}
