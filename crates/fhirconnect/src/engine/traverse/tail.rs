// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A value read and written below the template's deepest node, one tail
//! attribute at a time.

use fhir_types::codec::Value;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::paths::FlatId;
use openehr_rm::v1_2::model;
use openehr_rm::v1_2::paths::item_at_path;

use crate::engine::cell;
use crate::engine::family;
use crate::engine::fhir::FhirValue;
use crate::engine::outcome::Warning;
use crate::engine::rm;
use crate::engine::rm::Carried;
use crate::engine::rm::RmError;
use crate::engine::rm::RmValue;
use crate::resolve::program::mapping::Mapping;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;

use crate::tree::element::Table;

use crate::engine::traverse::Binding;
use crate::engine::traverse::Held;
use crate::engine::traverse::Run;
use crate::engine::traverse::Written;
use crate::engine::traverse::error::EngineError;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Writes one FHIR element into the attribute a tail names below a node.
    ///
    /// The tail's leaf class selects the cell when the leaf is a data value,
    /// and a scalar leaf takes the text of the FHIR element. The attribute is
    /// merged into what the place already holds, so several mappings that
    /// name attributes of one value build that one value, and the whole value
    /// is read as its class when the composition is built.
    pub(super) fn convert_tail(
        &mut self,
        mapping: &Mapping,
        target: &FhirTarget,
        openehr: &OpenehrTarget,
        view: &FhirValue,
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let node = openehr.node();
        let segments = tail_segments(openehr);
        let (class, carried) = rm::carried(node.rm_type(), &segments)
            .ok_or_else(|| unsupported_tail(mapping, openehr))?;
        let refuse_rm = |source: RmError| EngineError::Rm {
            mapping: String::from(mapping.name()),
            node: String::from(node.aql_path().as_str()),
            source: Box::new(source),
        };
        if carried == Carried::Family {
            let leaf = openehr
                .leaf_class()
                .ok_or_else(|| unsupported_tail(mapping, openehr))?;
            let produced = cell::put(view, leaf, None, self.binding_of(node).as_deref()).map_err(
                |source| EngineError::Cell {
                    mapping: String::from(mapping.name()),
                    element: String::from(target.expression().as_str()),
                    source: Box::new(source),
                },
            )?;
            for taken in &produced.fallbacks {
                self.warnings.push(Warning::fallback(taken));
            }
            let RmValue::Party(ref party) = produced.value else {
                return Err(refuse_rm(RmError::UnknownClass {
                    rm_type: String::from(produced.value.rm_type()),
                }));
            };
            self.families.extend(family::provider(
                node,
                &binding.openehr,
                &rm::family_of(&segments),
                party,
            ));
            return Ok(());
        }
        let mut object = self
            .held_object(&binding.openehr, node.flat_id())
            .map_err(refuse_rm)?;
        let written = if carried == Carried::Value {
            let leaf = openehr
                .leaf_class()
                .ok_or_else(|| unsupported_tail(mapping, openehr))?;
            // NOTE: no specification governs this: our own design, a held
            // sub-value still being written attribute by attribute is no whole
            // value yet, so it carries nothing over and the cell starts fresh.
            let held = serde_json::Value::Object(object.clone());
            let existing = item_at_path(&held, openehr.tail()).and_then(|found| {
                RmValue::from_canonical(leaf, node.aql_path().as_str(), found).ok()
            });
            let produced = cell::put(
                view,
                leaf,
                existing.as_ref(),
                self.binding_of(node).as_deref(),
            )
            .map_err(|source| EngineError::Cell {
                mapping: String::from(mapping.name()),
                element: String::from(target.expression().as_str()),
                source: Box::new(source),
            })?;
            for taken in &produced.fallbacks {
                self.warnings.push(Warning::fallback(taken));
            }
            produced.value.to_canonical().map_err(refuse_rm)?
        } else {
            let element = String::from(target.resolved().leaf());
            let text = view.write(&element).map_err(|source| EngineError::Fhir {
                mapping: String::from(mapping.name()),
                element: element.clone(),
                source: Box::new(source),
            })?;
            scalar_json(carried, class, node, &text).map_err(refuse_rm)?
        };
        merge_value(&mut object, &segments, written);
        let kept = object
            .get("_type")
            .and_then(serde_json::Value::as_str)
            .filter(|held| *held == class || model::is_a(held, class))
            .map(String::from);
        object.insert(
            String::from("_type"),
            serde_json::Value::String(kept.unwrap_or_else(|| String::from(class))),
        );
        self.store(Written {
            flat_id: node.flat_id().clone(),
            positions: binding.openehr.clone(),
            value: Held::Partial(object),
        });
        Ok(())
    }

    /// Returns the canonical JSON the run holds at one place, empty when it
    /// holds nothing there.
    fn held_object(
        &self,
        positions: &[RmPosition],
        flat_id: &FlatId,
    ) -> Result<serde_json::Map<String, serde_json::Value>, RmError> {
        let Some(written) = self
            .written
            .iter()
            .find(|written| written.flat_id == *flat_id && written.positions == positions)
        else {
            return Ok(serde_json::Map::new());
        };
        match written.value {
            Held::Partial(ref object) => Ok(object.clone()),
            Held::Value(ref value) => match value.to_canonical()? {
                serde_json::Value::Object(object) => Ok(object),
                _ => Ok(serde_json::Map::new()),
            },
        }
    }
}

/// Returns the value a target's tail names below its node.
///
/// A Web Template node is as deep as the template constrains, and a target
/// may name an attribute below it, so the tail is read from the canonical
/// JSON of the node with `PATHABLE.item_at_path` of `openehr-rm`.
pub(super) fn attribute_at<'value>(
    value: &'value serde_json::Value,
    target: &OpenehrTarget,
) -> Option<&'value serde_json::Value> {
    item_at_path(value, target.tail())
}

/// Returns the text one FHIR value is compared by, for a `unique` tuple.
pub(super) fn lexical(value: &Value) -> String {
    match *value {
        Value::Null => String::new(),
        Value::Bool(flag) => String::from(if flag { "true" } else { "false" }),
        Value::Number(ref number) => String::from(number.as_str()),
        Value::String(ref text) => text.clone(),
        Value::Array(ref items) => items.iter().map(lexical).collect::<Vec<String>>().join(","),
        Value::Object(ref object) => object
            .iter()
            .map(|(key, member)| format!("{key}={}", lexical(member)))
            .collect::<Vec<String>>()
            .join(";"),
    }
}

/// Returns the attribute names a target's tail walks, outermost first.
pub(super) fn tail_segments(target: &OpenehrTarget) -> Vec<&str> {
    target
        .tail()
        .segments
        .iter()
        .map(|segment| segment.attribute.as_str())
        .collect()
}

/// Returns the refusal of a tail no FLAT part of the node's class carries.
pub(super) fn unsupported_tail(mapping: &Mapping, target: &OpenehrTarget) -> EngineError {
    EngineError::UnsupportedTail {
        mapping: String::from(mapping.name()),
        node: String::from(target.node().aql_path().as_str()),
        tail: target.tail().to_string(),
    }
}

/// Merges one value into an object at an attribute path.
///
/// An intermediate attribute that holds no object is replaced by one, since
/// a path below it names its members.
fn merge_value(
    object: &mut serde_json::Map<String, serde_json::Value>,
    segments: &[&str],
    value: serde_json::Value,
) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        object.insert(String::from(*first), value);
        return;
    }
    let entry = object
        .entry(String::from(*first))
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !entry.is_object() {
        *entry = serde_json::Value::Object(serde_json::Map::new());
    }
    if let serde_json::Value::Object(ref mut nested) = *entry {
        merge_value(nested, rest, value);
    }
}

/// Returns the canonical JSON of one scalar tail attribute, from the FHIR
/// element's JSON.
///
/// The attribute's type comes from the RM attribute model, so a text that
/// does not read as it is a refusal naming the class.
fn scalar_json(
    carried: Carried,
    class: &str,
    node: &ResolvedNode,
    written: &Value,
) -> Result<serde_json::Value, RmError> {
    let text = match *written {
        Value::String(ref text) => text.clone(),
        Value::Number(ref number) => String::from(number.as_str()),
        Value::Bool(flag) => String::from(if flag { "true" } else { "false" }),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    let refuse = |expected: &'static str| RmError::Scalar {
        node: String::from(node.aql_path().as_str()),
        rm_type: String::from(class),
        text: text.clone(),
        expected,
    };
    match carried {
        Carried::Text if written.as_str().is_some() => Ok(serde_json::Value::String(text)),
        Carried::Text | Carried::Value | Carried::Family => Err(refuse("text")),
        Carried::Real => serde_json::from_str::<serde_json::Number>(&text)
            .map(serde_json::Value::Number)
            .map_err(|_refused| refuse("a real number")),
        Carried::Integer => text
            .parse::<i64>()
            .map(|number| serde_json::Value::Number(number.into()))
            .map_err(|_refused| refuse("an integer")),
        Carried::Boolean => match text.as_str() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(refuse("a boolean")),
        },
    }
}

/// Returns the text a scalar carries, `None` for a structure.
pub(super) fn scalar(value: &serde_json::Value) -> Option<String> {
    match *value {
        serde_json::Value::String(ref text) => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(String::from(if flag { "true" } else { "false" })),
        serde_json::Value::Number(ref number) => Some(number.to_string()),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
}

/// Merges one manual path into the object the entry builds.
pub(super) fn merge(
    object: &mut serde_json::Map<String, serde_json::Value>,
    segments: &[&str],
    value: &str,
) {
    let Some((first, rest)) = segments.split_first() else {
        object.insert(
            String::from("value"),
            serde_json::Value::String(String::from(value)),
        );
        return;
    };
    if rest.is_empty() {
        object.insert(
            String::from(*first),
            serde_json::Value::String(String::from(value)),
        );
        return;
    }
    let entry = object
        .entry(String::from(*first))
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(ref mut nested) = *entry {
        merge(nested, rest, value);
    }
}

#[cfg(test)]
mod tests {
    use super::merge;

    #[test]
    fn manual_paths_that_share_a_prefix_merge_into_one_element() {
        // manual.adoc: "`defining_code/terminology_id/value` and
        // `defining_code/code_string` are merged together in the same data
        // element and do not overwrite".
        let mut object = serde_json::Map::new();
        merge(
            &mut object,
            &["defining_code", "terminology_id", "value"],
            "openehr",
        );
        merge(&mut object, &["defining_code", "code_string"], "524");
        merge(&mut object, &["value"], "Initial");
        assert_eq!(
            serde_json::Value::Object(object),
            serde_json::json!({
                "defining_code": {"terminology_id": {"value": "openehr"}, "code_string": "524"},
                "value": "Initial"
            })
        );
    }
}
