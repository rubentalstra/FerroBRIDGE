// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pairing of a contribution's versions with the entries sent, and the
//! tie-break on content when two entries share one `FEEDER_AUDIT` key.

use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::ids::ContributionUid;

use super::Mapped;
use super::Refused;
use super::Scope;
use super::refusal::unbound;

/// What names the item one composition was mapped from: the
/// `originating_system_item_ids[0]` type and id and the
/// `originating_system_audit.version_id` the engine writes into its
/// `FEEDER_AUDIT`
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_feeder_audit_class>).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AuditKey {
    /// The item's type.
    item_type: Option<String>,
    /// The item's id.
    item_id: Option<String>,
    /// The item's version.
    version_id: Option<String>,
}

impl AuditKey {
    /// Returns the key the `FEEDER_AUDIT` of `composition` states.
    fn of(composition: &Composition) -> Self {
        let audit = composition.feeder_audit.as_ref();
        let item = audit
            .and_then(|audit| audit.originating_system_item_ids.as_deref())
            .and_then(<[_]>::first);
        Self {
            item_type: item.and_then(|item| item.r#type.clone()),
            item_id: item.map(|item| item.id.clone()),
            version_id: audit.and_then(|audit| audit.originating_system_audit.version_id.clone()),
        }
    }
}

/// Returns the JSON pointer of every `DV_DATE_TIME` in `composition` whose
/// value is `instant`, the clock reading the run defaulted from.
///
/// The engine hands its one instant to the Simplified Formats builder as the
/// `ctx/time` default, which fills `EVENT_CONTEXT.start_time`,
/// `HISTORY.origin`, `EVENT.time` and `ACTION.time` wherever no mapping wrote
/// them (master06 §time), and the builder copies the instant verbatim, so a
/// value equal to it is one the clock supplied.
fn clock_filled(composition: &serde_json::Value, instant: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(String::new(), composition)];
    while let Some((pointer, node)) = pending.pop() {
        match node {
            serde_json::Value::Object(members) => {
                let is_instant = members.get("_type").and_then(serde_json::Value::as_str)
                    == Some("DV_DATE_TIME")
                    && members.get("value").and_then(serde_json::Value::as_str) == Some(instant);
                if is_instant {
                    found.push(pointer);
                    continue;
                }
                for (name, member) in members {
                    let step = name.replace('~', "~0").replace('/', "~1");
                    pending.push((format!("{pointer}/{step}"), member));
                }
            }
            serde_json::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    pending.push((format!("{pointer}/{index}"), item));
                }
            }
            _ => {}
        }
    }
    found.sort();
    found
}

/// Whether the composition `sent`, mapped at `instant`, and a stored one
/// carry the same content.
///
/// The comparison masks what differs between two mappings of one resource and
/// is no content of the source: the `uid` the CDR assigns, and every time the
/// run filled from its clock ([`clock_filled`]), whatever the stored
/// composition holds at that place.
pub(super) fn same_content(sent: &Composition, instant: &str, stored: &Composition) -> bool {
    let canonical = |composition: &Composition| {
        let mut composition = composition.clone();
        composition.uid = None;
        openehr_its::json::to_canonical_json(&composition).parse::<serde_json::Value>()
    };
    // NOTE: no specification governs this: our own design; a composition that
    // does not read back as JSON cannot be compared, so it matches nothing.
    let (Ok(mut sent), Ok(mut held)) = (canonical(sent), canonical(stored)) else {
        return false;
    };
    for pointer in clock_filled(&sent, instant) {
        for tree in [&mut sent, &mut held] {
            if let Some(slot) = tree.pointer_mut(&pointer) {
                *slot = serde_json::Value::Null;
            }
        }
    }
    sent == held
}

/// Returns the entry each committed version is the composition of, one
/// version per entry with its stored composition, in entry order.
///
/// `stored` is each version the contribution named with the composition the
/// CDR holds for it. A version matches an entry when its `FEEDER_AUDIT` names
/// the entry's item ([`AuditKey`]); where several entries name one item, as
/// the entries of one message do, it must also carry the entry's content
/// ([`same_content`]: the CDR-assigned `uid` and every time the run filled
/// from its clock are masked, so a re-sent Bundle mapped at another instant
/// still matches). A version that matches more than one entry, or an entry
/// two versions match or none, refuses the whole answer, and so does a
/// version that matches no entry under [`Scope::Whole`]. No specification
/// governs the matching: our own design.
pub(super) fn pair(
    contribution: &ContributionUid,
    mapped: &[Mapped<'_>],
    stored: Vec<(ObjectVersionId, Composition)>,
    scope: Scope,
) -> Result<Vec<(ObjectVersionId, Composition)>, Refused> {
    let keys: Vec<AuditKey> = mapped
        .iter()
        .map(|entry| AuditKey::of(&entry.composition))
        .collect();
    let mut placed: Vec<Option<(ObjectVersionId, Composition)>> = vec![None; mapped.len()];
    for (version, composition) in stored {
        let key = AuditKey::of(&composition);
        let sharing = keys.iter().filter(|candidate| **candidate == key).count();
        let matching: Vec<usize> = mapped
            .iter()
            .zip(&keys)
            .enumerate()
            .filter(|(_, (entry, candidate))| {
                **candidate == key
                    && (sharing == 1
                        || same_content(&entry.composition, &entry.instant, &composition))
            })
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() && scope == Scope::Part {
            continue;
        }
        let [index] = matching.as_slice() else {
            return Err(unbound(
                contribution,
                &format!(
                    "its version {} matches {} of the entries sent, where it must match one",
                    version.value(),
                    matching.len()
                ),
            ));
        };
        let slot = placed.get_mut(*index).ok_or_else(|| {
            unbound(
                contribution,
                &format!("its version {} matches no entry", version.value()),
            )
        })?;
        if slot.is_some() {
            let full_url = mapped
                .get(*index)
                .map_or("an entry", |entry| entry.full_url.as_str());
            return Err(unbound(
                contribution,
                &format!("two of its versions match {full_url}"),
            ));
        }
        *slot = Some((version, composition));
    }
    placed
        .into_iter()
        .zip(mapped)
        .map(|(slot, entry)| {
            slot.ok_or_else(|| {
                unbound(
                    contribution,
                    &format!("none of its versions matches {}", entry.full_url),
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clock_mask_names_every_date_time_at_the_instant_and_nothing_else() {
        let instant = "2026-09-25T10:00:00.123456789Z";
        let composition = serde_json::json!({
            "_type": "COMPOSITION",
            "context": {
                "_type": "EVENT_CONTEXT",
                "start_time": { "_type": "DV_DATE_TIME", "value": instant }
            },
            "content": [{
                "_type": "OBSERVATION",
                "data": {
                    "_type": "HISTORY",
                    "origin": { "_type": "DV_DATE_TIME", "value": instant },
                    "events": [{
                        "_type": "POINT_EVENT",
                        "time": { "_type": "DV_DATE_TIME", "value": "2026-09-13T10:00:00+02:00" }
                    }]
                },
                "name": { "_type": "DV_TEXT", "value": instant }
            }]
        });
        assert_eq!(
            vec![
                String::from("/content/0/data/origin"),
                String::from("/context/start_time"),
            ],
            super::clock_filled(&composition, instant),
            "a mapped time and a text that reads like the instant stay compared"
        );
    }
}
