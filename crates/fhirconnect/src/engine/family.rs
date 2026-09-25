// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reference-model attribute families a composition carries beside its
//! archetyped content.
//!
//! `FEEDER_AUDIT`, `LINK` and `PARTICIPATION` are attributes of the reference
//! model that no Web Template node stands for, so the composition seam reaches
//! them as the `_`-prefixed families Simplified Formats spells under the node
//! they belong to: `_feeder_audit` on any `LOCATABLE`, `_link:i` on any
//! `LOCATABLE`, `_other_participation:i` on an `ENTRY` and `_participation:i`
//! under the `context` of a composition (openEHR ITS-REST 1.1.0, Simplified
//! Formats, `docs/specs/its-rest/docs/simplified_formats/master05-rm_mapping.adoc`,
//! the sections `EVENT_CONTEXT`, the `ENTRY` classes, `LINK`, `FEEDER_AUDIT`
//! and `PARTICIPATION`). This module writes those keys as [`NodeValue`]s and
//! reads the same attributes back out of the canonical JSON of the node.

use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::ResolvedNode;

use crate::engine::origin::Origin;
use crate::engine::origin::UNKNOWN_SOURCE;

/// The `feeder_system_item_ids` type every defaulted field is recorded under.
///
/// No specification governs the spelling: our own design.
pub const DEFAULTED: &str = "defaulted";

/// Returns the `_feeder_audit` family of the composition root.
///
/// `originating_system_audit.system_id` and `feeder_system_audit.system_id`
/// name the bridge, the source resource travels as the one
/// `originating_system_item_ids` entry with its `meta.versionId` as
/// `originating_system_audit.version_id`, and each defaulted field travels as
/// one `feeder_system_item_ids` entry whose `id` is the field's openEHR path,
/// in `defaulted` order
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_feeder_audit_class>).
/// The allocation of facts to attributes is our own design.
#[must_use]
pub fn feeder_audit(root: &ResolvedNode, origin: &Origin, defaulted: &[String]) -> Vec<NodeValue> {
    let family = String::from("_feeder_audit");
    let at = |segments: &[String], datum: &str, value: &str| {
        let mut sub_path = vec![family.clone()];
        sub_path.extend(segments.iter().cloned());
        NodeValue::new(root, serde_json::Value::String(String::from(value)))
            .under(sub_path)
            .with_datum(datum)
    };
    let originating = [String::from("originating_system_audit")];
    let feeder = [String::from("feeder_system_audit")];
    let mut values = vec![at(&originating, "system_id", origin.system_id())];
    if let Some(source) = origin.source() {
        if let Some(version) = source.version_id() {
            values.push(at(&originating, "version_id", version));
        }
        let item = [String::from("originating_system_item_id:0")];
        values.push(at(&item, "id", source.id().unwrap_or(UNKNOWN_SOURCE)));
        values.push(at(&item, "type", source.resource_type()));
    }
    if defaulted.is_empty() {
        return values;
    }
    values.push(at(&feeder, "system_id", origin.system_id()));
    for (index, field) in defaulted.iter().enumerate() {
        let item = [format!("feeder_system_item_id:{index}")];
        values.push(at(&item, "id", field));
        values.push(at(&item, "type", DEFAULTED));
        values.push(at(&item, "issuer", origin.system_id()));
    }
    values
}

/// Returns the `_link:i` family of one `LINK` on `node`.
///
/// `LINK.meaning` and `LINK.type` are `DV_TEXT` and `LINK.target` a
/// `DV_EHR_URI`, all three mandatory
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_link_class>).
#[must_use]
pub fn link(
    node: &ResolvedNode,
    positions: &[RmPosition],
    index: usize,
    parts: &LinkParts<'_>,
) -> Vec<NodeValue> {
    let family = vec![format!("_link:{index}")];
    [
        ("meaning", parts.meaning),
        ("type", parts.link_type),
        ("target", parts.target),
    ]
    .into_iter()
    .map(|(datum, value)| {
        NodeValue::new(node, serde_json::Value::String(String::from(value)))
            .with_occurrences(positions.to_vec())
            .under(family.clone())
            .with_datum(datum)
    })
    .collect()
}

/// The three attributes of one `LINK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkParts<'text> {
    /// `LINK.meaning`.
    pub meaning: &'text str,
    /// `LINK.type`.
    pub link_type: &'text str,
    /// `LINK.target`, an `ehr:` URI.
    pub target: &'text str,
}

/// The attributes of one `PARTICIPATION` the engine writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParticipationParts<'text> {
    /// `PARTICIPATION.function`, a `DV_TEXT`.
    pub function: &'text str,
    /// The performer's `PARTY_IDENTIFIED.name`, when the reference has one.
    pub name: Option<&'text str>,
    /// The performer's `external_ref.id.value`.
    pub id: &'text str,
    /// The performer's `external_ref.id.scheme`.
    pub id_scheme: &'text str,
    /// The performer's `external_ref.namespace`.
    pub id_namespace: &'text str,
}

/// One list of `PARTICIPATION`s the reference model gives a class, with the
/// FLAT family it travels as.
///
/// `ENTRY.other_participations` and `EVENT_CONTEXT.participations` are the
/// same `List<PARTICIPATION>`
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_entry_class>,
/// <https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html#_event_context_class>),
/// spelled `_other_participation:i` and `_participation:i` (`master05-rm_mapping.adoc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParticipationList {
    /// The owner class the list belongs to, or an ancestor of it.
    pub owner: &'static str,
    /// The reference-model attribute that holds the list.
    pub attribute: &'static str,
    /// The FLAT family one participation of it is written under.
    pub family: &'static str,
}

/// The participations of an `ENTRY`.
pub const ENTRY_PARTICIPATIONS: ParticipationList = ParticipationList {
    owner: "ENTRY",
    attribute: "other_participations",
    family: "_other_participation",
};

/// The participations of an `EVENT_CONTEXT`.
pub const CONTEXT_PARTICIPATIONS: ParticipationList = ParticipationList {
    owner: "EVENT_CONTEXT",
    attribute: "participations",
    family: "_participation",
};

/// Returns the participation list `tail` names below a node of `class`.
#[must_use]
pub fn participation_list(class: &str, tail: &[&str]) -> Option<ParticipationList> {
    [ENTRY_PARTICIPATIONS, CONTEXT_PARTICIPATIONS]
        .into_iter()
        .find(|list| tail == [list.attribute] && openehr_rm::v1_2::model::is_a(class, list.owner))
}

/// Returns the family of one `PARTICIPATION` of `list` on `node`.
///
/// The performer is inlined as `|name`, `|id`, `|id_scheme` and
/// `|id_namespace` on the participation itself (Simplified Formats, the
/// `PARTICIPATION` mapping table).
#[must_use]
pub fn participation(
    node: &ResolvedNode,
    list: ParticipationList,
    positions: &[RmPosition],
    index: usize,
    parts: &ParticipationParts<'_>,
) -> Vec<NodeValue> {
    let family = vec![format!("{}:{index}", list.family)];
    let mut data = vec![("function", parts.function)];
    if let Some(name) = parts.name {
        data.push(("name", name));
    }
    data.push(("id", parts.id));
    data.push(("id_scheme", parts.id_scheme));
    data.push(("id_namespace", parts.id_namespace));
    data.into_iter()
        .map(|(datum, value)| {
            NodeValue::new(node, serde_json::Value::String(String::from(value)))
                .with_occurrences(positions.to_vec())
                .under(family.clone())
                .with_datum(datum)
        })
        .collect()
}

/// Returns the `LINK.target` values of the links a node carries with one
/// meaning and one type, in order.
///
/// `value` is the canonical JSON of the node, whose `links` is the
/// `LOCATABLE` attribute; the meaning and the type are what tell one `link`
/// mapping's links from another's.
#[must_use]
pub fn link_targets(value: &serde_json::Value, meaning: &str, link_type: &str) -> Vec<String> {
    let text = |link: &serde_json::Value, attribute: &str| {
        link.get(attribute)
            .and_then(|text| text.get("value"))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    };
    value
        .get("links")
        .and_then(serde_json::Value::as_array)
        .map(|links| {
            links
                .iter()
                .filter(|link| {
                    text(link, "meaning").as_deref() == Some(meaning)
                        && text(link, "type").as_deref() == Some(link_type)
                })
                .filter_map(|link| text(link, "target"))
                .collect()
        })
        .unwrap_or_default()
}

/// One `PARTICIPATION` read back out of an `ENTRY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participation {
    /// The performer's `external_ref.id.value`, when it carries one.
    pub id: Option<String>,
    /// The performer's name, when it carries one.
    pub name: Option<String>,
}

/// Returns the participations of `list` whose function is `function`.
///
/// `value` is the canonical JSON of the node that holds the list
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_participation_class>).
#[must_use]
pub fn participations(
    value: &serde_json::Value,
    list: ParticipationList,
    function: &str,
) -> Vec<Participation> {
    value
        .get(list.attribute)
        .and_then(serde_json::Value::as_array)
        .map(|found| {
            found
                .iter()
                .filter(|entry| {
                    entry
                        .get("function")
                        .and_then(|text| text.get("value"))
                        .and_then(serde_json::Value::as_str)
                        == Some(function)
                })
                .map(|entry| {
                    let performer = entry.get("performer");
                    Participation {
                        id: performer
                            .and_then(|party| party.get("external_ref"))
                            .and_then(|reference| reference.get("id"))
                            .and_then(|id| id.get("value"))
                            .and_then(serde_json::Value::as_str)
                            .map(String::from),
                        name: performer
                            .and_then(|party| party.get("name"))
                            .and_then(serde_json::Value::as_str)
                            .map(String::from),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
