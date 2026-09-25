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
//! reads the same attributes back out of the canonical JSON of the node as
//! typed `openehr-rm` values.

// TODO(#241): the write side spells the `_feeder_audit`, `_link:i`,
// `_participation:i`, `_other_participation:i` and `_provider` families key by
// key until openehr-sdt honours `|raw` on `_`-prefixed attribute families or
// makes `emit_rm_attrs` public (sibling request S1).

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_its::json::JsonParseError;
use openehr_its::json::from_canonical_value;
use openehr_mapping_core::composition::NodeValue;
use openehr_mapping_core::composition::RmPosition;
use openehr_mapping_core::index::ResolvedNode;
use openehr_rm::v1_2::common::archetyped::link::Link;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentifiedData;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use openehr_sdt::flat::path::Segment;

use crate::engine::origin::Origin;
use crate::engine::origin::UNKNOWN_SOURCE;

/// The `feeder_system_item_ids` type every defaulted field is recorded under.
///
/// No specification governs the spelling: our own design.
pub const DEFAULTED: &str = "defaulted";

/// Why a family a node carries could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the `{attribute}` of the node does not read as its reference-model class")]
pub struct FamilyError {
    /// The reference-model attribute that holds the family.
    pub attribute: &'static str,
    /// What the strict ITS-JSON reader refused, with its JSON path.
    #[source]
    pub source: JsonParseError,
}

/// Returns a segment with no instance index.
fn plain(name: &str) -> Segment {
    Segment {
        name: String::from(name),
        index: None,
    }
}

/// Returns a segment with the instance index `index`.
///
/// NOTE: no specification governs this: our own design, an index past `u32`
/// saturates, and the composition seam refuses it past the FLAT bound.
fn indexed(name: &str, index: usize) -> Segment {
    Segment {
        name: String::from(name),
        index: Some(u32::try_from(index).unwrap_or(u32::MAX)),
    }
}

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
    let at = |segment: Segment, datum: &str, value: &str| {
        NodeValue::new(root, serde_json::Value::String(String::from(value)))
            .under(vec![plain("_feeder_audit"), segment])
            .with_datum(datum)
    };
    let originating = || plain("originating_system_audit");
    let mut values = vec![at(originating(), "system_id", origin.system_id())];
    if let Some(source) = origin.source() {
        if let Some(version) = source.version_id() {
            values.push(at(originating(), "version_id", version));
        }
        let item = || indexed("originating_system_item_id", 0);
        values.push(at(item(), "id", source.id().unwrap_or(UNKNOWN_SOURCE)));
        values.push(at(item(), "type", source.resource_type()));
    }
    if defaulted.is_empty() {
        return values;
    }
    values.push(at(
        plain("feeder_system_audit"),
        "system_id",
        origin.system_id(),
    ));
    for (index, field) in defaulted.iter().enumerate() {
        let item = || indexed("feeder_system_item_id", index);
        values.push(at(item(), "id", field));
        values.push(at(item(), "type", DEFAULTED));
        values.push(at(item(), "issuer", origin.system_id()));
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
    [
        ("meaning", parts.meaning),
        ("type", parts.link_type),
        ("target", parts.target),
    ]
    .into_iter()
    .map(|(datum, value)| {
        NodeValue::new(node, serde_json::Value::String(String::from(value)))
            .with_occurrences(positions.to_vec())
            .under(vec![indexed("_link", index)])
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
                .under(vec![indexed(list.family, index)])
                .with_datum(datum)
        })
        .collect()
}

/// Returns the family of one `PARTY_IDENTIFIED` written under `family` on
/// `node`, the `_provider` of an `ENTRY`.
///
/// The party is `|name` plus one `_identifier:i` per identifier, each with
/// `|id`, `|issuer`, `|assigner` and `|type` (Simplified Formats, the
/// `PARTY_IDENTIFIED` and `DV_IDENTIFIER` tables).
#[must_use]
pub fn provider(
    node: &ResolvedNode,
    positions: &[RmPosition],
    family: &[Segment],
    party: &PartyIdentifiedData,
) -> Vec<NodeValue> {
    let at = |below: Vec<Segment>, datum: &str, value: &str| {
        let mut sub_path = family.to_vec();
        sub_path.extend(below);
        NodeValue::new(node, serde_json::Value::String(String::from(value)))
            .with_occurrences(positions.to_vec())
            .under(sub_path)
            .with_datum(datum)
    };
    let mut values = Vec::new();
    if let Some(ref name) = party.name {
        values.push(at(Vec::new(), "name", name));
    }
    for (index, identifier) in party
        .identifiers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let below = || vec![indexed("_identifier", index)];
        values.push(at(below(), "id", &identifier.id));
        for (datum, value) in [
            ("issuer", identifier.issuer.as_deref()),
            ("assigner", identifier.assigner.as_deref()),
            ("type", identifier.r#type.as_deref()),
        ] {
            if let Some(value) = value {
                values.push(at(below(), datum, value));
            }
        }
    }
    values
}

/// Returns the refusal of a present list `attribute` that does not decode.
///
/// Both lists are optional in the reference model (`LOCATABLE.links`,
/// `ENTRY.other_participations` and `EVENT_CONTEXT.participations` are
/// `0..1`), so an absent attribute reads as no entries; a present one that
/// does not decode is a defect of the document and a refusal.
const fn undecodable(attribute: &'static str) -> impl Fn(JsonParseError) -> FamilyError {
    move |source| FamilyError { attribute, source }
}

/// Returns the text a `DV_TEXT` or `DV_CODED_TEXT` carries.
fn text_of(text: &DvText) -> &str {
    match *text {
        DvText::DvCodedText(ref coded) => &coded.value,
        DvText::DvText(ref plain) => &plain.value,
    }
}

/// Returns the `LINK.target` values of the links a node carries with one
/// meaning and one type, in order.
///
/// `value` is the canonical JSON of the node, whose `links` is the
/// `LOCATABLE` attribute; the meaning and the type are what tell one `link`
/// mapping's links from another's.
///
/// # Errors
///
/// Returns [`FamilyError`] when the node's `links` do not decode as `LINK`s.
pub fn link_targets(
    value: &serde_json::Value,
    meaning: &str,
    link_type: &str,
) -> Result<Vec<String>, FamilyError> {
    let Some(found) = value.get("links") else {
        return Ok(Vec::new());
    };
    Ok(from_canonical_value::<Vec<Link>>(found)
        .map_err(undecodable("links"))?
        .into_iter()
        .filter(|link| text_of(&link.meaning) == meaning && text_of(&link.r#type) == link_type)
        .map(|link| link.target.value)
        .collect())
}

/// One `PARTICIPATION` read back out of an `ENTRY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participation {
    /// The performer's `external_ref.id.value`, when it carries one.
    pub id: Option<String>,
    /// The performer's name, when it carries one.
    pub name: Option<String>,
}

/// Returns the value of an object id, whichever subtype it is.
fn id_value(id: &ObjectId) -> String {
    match *id {
        ObjectId::ArchetypeId(ref id) => id.value.clone(),
        ObjectId::GenericId(ref id) => id.value.clone(),
        ObjectId::HierObjectId(ref id) => String::from(id.value()),
        ObjectId::ObjectVersionId(ref id) => String::from(id.value()),
        ObjectId::TemplateId(ref id) => id.value.clone(),
        ObjectId::TerminologyId(ref id) => id.value.clone(),
    }
}

/// Returns the participations of `list` whose function is `function`.
///
/// `value` is the canonical JSON of the node that holds the list
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_participation_class>).
///
/// # Errors
///
/// Returns [`FamilyError`] when the list does not decode as `PARTICIPATION`s.
pub fn participations(
    value: &serde_json::Value,
    list: ParticipationList,
    function: &str,
) -> Result<Vec<Participation>, FamilyError> {
    let Some(found) = value.get(list.attribute) else {
        return Ok(Vec::new());
    };
    Ok(from_canonical_value::<
        Vec<openehr_rm::v1_2::common::generic::participation::Participation>,
    >(found)
    .map_err(undecodable(list.attribute))?
    .into_iter()
        .filter(|entry| text_of(&entry.function) == function)
        .map(|entry| {
            let (reference, name) = match entry.performer {
                PartyProxy::PartySelf(party) => (party.external_ref, None),
                PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) => {
                    (party.external_ref, party.name)
                }
                PartyProxy::PartyIdentified(PartyIdentified::PartyRelated(party)) => {
                    (party.external_ref, party.name)
                }
            };
            Participation {
                id: reference.map(|reference| id_value(&reference.id)),
                name,
            }
        })
        .collect())
}
