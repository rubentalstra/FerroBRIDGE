// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The declared set of a round trip, the one comparison every round-trip
//! suite asserts.
//!
//! A mapping program declares what it skips and what it defaults, and an
//! element no mapping reaches does not survive a round trip. So a round trip
//! is compared leaf by leaf: every scalar of the input and of the output is
//! keyed by its path, and what the output lacks, carries extra or carries
//! differently is listed beside the losses the two legs declared. A suite pins
//! that set, and the round trip holds when the output equals the input modulo
//! exactly it. No specification governs this: our own design.

use std::collections::BTreeMap;

/// Returns every scalar of `document` keyed by its path.
///
/// An object member extends the path with `.name`, an array item with
/// `[index]`, and a scalar is kept as its JSON text, so a string and a number
/// of the same spelling stay apart.
#[must_use]
pub fn leaves(document: &serde_json::Value) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    walk(document, String::new(), &mut found);
    found
}

/// Records the scalars below `value` under `path`.
fn walk(value: &serde_json::Value, path: String, found: &mut BTreeMap<String, String>) {
    match *value {
        serde_json::Value::Object(ref members) => {
            for (key, member) in members {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                walk(member, next, found);
            }
        }
        serde_json::Value::Array(ref items) => {
            for (index, item) in items.iter().enumerate() {
                walk(item, format!("{path}[{index}]"), found);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {
            found.insert(path, value.to_string());
        }
    }
}

/// Returns the declared set of one round trip.
///
/// `first` and `second` are the losses each leg declared, in the order it
/// took them; `input` and `output` are the two documents the round trip
/// compares. The answer carries five lists: `first_leg`, `second_leg`, `lost`
/// (leaves only the input carries), `added` (leaves only the output carries)
/// and `changed` (leaves both carry with different values).
#[must_use]
pub fn declared(
    first: &[String],
    second: &[String],
    input: &serde_json::Value,
    output: &serde_json::Value,
) -> serde_json::Value {
    let before = leaves(input);
    let after = leaves(output);
    let lost: Vec<String> = before
        .iter()
        .filter(|&(path, _)| !after.contains_key(path))
        .map(|(path, value)| format!("{path} = {value}"))
        .collect();
    let added: Vec<String> = after
        .iter()
        .filter(|&(path, _)| !before.contains_key(path))
        .map(|(path, value)| format!("{path} = {value}"))
        .collect();
    let changed: Vec<String> = before
        .iter()
        .filter_map(|(path, old)| {
            after
                .get(path)
                .filter(|new| *new != old)
                .map(|new| format!("{path}: {old} became {new}"))
        })
        .collect();
    serde_json::json!({
        "first_leg": first,
        "second_leg": second,
        "lost": lost,
        "added": added,
        "changed": changed,
    })
}

/// Replaces the first string equal to `from` anywhere in `document`, and
/// answers whether it found one.
///
/// A round-trip suite corrupts the document between its two legs with this,
/// to show that the comparison notices; a corruption that finds nothing is
/// the suite's own failure, which is why the answer is returned.
pub fn replace_first(document: &mut serde_json::Value, from: &str, to: &str) -> bool {
    match *document {
        serde_json::Value::String(ref mut text) if text == from => {
            to.clone_into(text);
            true
        }
        serde_json::Value::Array(ref mut items) => {
            items.iter_mut().any(|item| replace_first(item, from, to))
        }
        serde_json::Value::Object(ref mut members) => members
            .values_mut()
            .any(|member| replace_first(member, from, to)),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{declared, leaves, replace_first};

    #[test]
    fn a_leaf_is_keyed_by_its_members_and_indices() {
        let found = leaves(&serde_json::json!({ "a": [{ "b": "x" }, 2] }));
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            vec![
                (String::from("a[0].b"), String::from("\"x\"")),
                (String::from("a[1]"), String::from("2")),
            ]
        );
    }

    #[test]
    fn an_identical_round_trip_declares_nothing_lost_added_or_changed() {
        let document = serde_json::json!({ "code": "c", "note": ["n"] });
        let set = declared(&[], &[], &document, &document);
        assert_eq!(set["lost"], serde_json::json!([]));
        assert_eq!(set["added"], serde_json::json!([]));
        assert_eq!(set["changed"], serde_json::json!([]));
    }

    #[test]
    fn each_difference_lands_in_its_own_list() {
        let set = declared(
            &[String::from("skipped")],
            &[],
            &serde_json::json!({ "kept": 1, "gone": "g", "moved": "a" }),
            &serde_json::json!({ "kept": 1, "new": "n", "moved": "b" }),
        );
        assert_eq!(set["first_leg"], serde_json::json!(["skipped"]));
        assert_eq!(set["lost"], serde_json::json!(["gone = \"g\""]));
        assert_eq!(set["added"], serde_json::json!(["new = \"n\""]));
        assert_eq!(
            set["changed"],
            serde_json::json!(["moved: \"a\" became \"b\""])
        );
    }

    #[test]
    fn a_replacement_that_finds_nothing_says_so() {
        let mut document = serde_json::json!({ "a": ["x"] });
        assert!(replace_first(&mut document, "x", "y"));
        assert_eq!(document, serde_json::json!({ "a": ["y"] }));
        assert!(!replace_first(&mut document, "x", "y"));
    }
}
