// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a version lists: its rooted structures, the withdrawals and the table
//! 0354 check.

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::legacy::lower::LegacyDefect;
use crate::v2::legacy::lower::LegacyError;
use crate::v2::legacy::lower::invalid;
use crate::v2::legacy::lower::tolerate;
use crate::v2::legacy::source::{
    CodeStatus, GroupRow, TABLE_0354, Table0354, Tables, VersionTables,
};
use crate::v2::lower::{DEFINITIONS_VERSION, Node};

/// Every segment id `nodes` place, at any depth.
pub(super) fn placed_segments<'n>(nodes: &'n [Node], out: &mut BTreeSet<&'n str>) {
    for node in nodes {
        match node {
            Node::Segment { segment, .. } => {
                out.insert(segment.as_str());
            }
            Node::Group { children, .. } => placed_segments(children, out),
            Node::Placeholder { .. } => {}
        }
    }
}

/// The versions that list each structure code `current` lacks, by the index
/// of the version, and the root group of each structure per version.
type Listing<'t> = (
    BTreeMap<String, Vec<usize>>,
    Vec<BTreeMap<String, &'t GroupRow>>,
);

pub(super) fn listing<'t>(
    tables: &'t Tables,
    current: &BTreeSet<&str>,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<Listing<'t>, LegacyError> {
    let mut listed: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut rooted = Vec::new();
    for (index, version) in tables.versions().iter().enumerate() {
        let roots = roots(version, hits)?;
        for message in &version.messages {
            // NOTE: no specification governs this: our own design; the tables list the
            // event-less general acknowledgment as `ACK` with an empty event.
            let expected = if message.event_id.is_empty() {
                message.msg_type_id.clone()
            } else {
                format!("{}_{}", message.msg_type_id, message.event_id)
            };
            if message.id != expected {
                return Err(invalid(
                    &version.file("messages"),
                    &message.id,
                    format!(
                        "names the code {} and event {}",
                        message.msg_type_id, message.event_id
                    ),
                ));
            }
            if !current.contains(message.id.as_str()) {
                listed.entry(message.id.clone()).or_default().push(index);
            }
            if !roots.contains_key(message.id.as_str()) {
                tolerate(
                    hits,
                    &version.file("messages"),
                    &message.id,
                    LegacyDefect::MessageWithoutTree,
                )?;
            }
        }
        rooted.push(roots);
    }
    Ok((listed, rooted))
}

/// The version each rooted code is withdrawn as of: the first version after
/// the last that lists it, among the export's versions and then v2.9.1, with
/// each code checked against table 0354.
pub(super) fn withdrawals(
    tables: &Tables,
    listed: &BTreeMap<String, Vec<usize>>,
    rooted: &[BTreeMap<String, &GroupRow>],
    table: &Table0354,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<BTreeMap<String, String>, LegacyError> {
    let mut order: Vec<&str> = tables
        .versions()
        .iter()
        .map(|version| version.version.as_str())
        .collect();
    order.push(DEFINITIONS_VERSION);
    let mut withdrawn = BTreeMap::new();
    for (code, indices) in listed {
        let carried = indices.iter().any(|index| {
            rooted
                .get(*index)
                .is_some_and(|roots| roots.contains_key(code.as_str()))
        });
        let Some(last) = indices.iter().copied().max().filter(|_| carried) else {
            continue;
        };
        check_table_0354(table, code, hits)?;
        let Some(next) = order.get(last.saturating_add(1)) else {
            return Err(invalid(TABLE_0354, code, "no version follows its last one"));
        };
        withdrawn.insert(code.clone(), (*next).to_owned());
    }
    Ok(withdrawn)
}

/// The structure code of a v2.9.1 structure id without its variant suffix:
/// `ORU_R01-A` is `ORU_R01`.
pub(super) fn base_code(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((base, variant)) if variant.len() == 1 => base,
        _ => id,
    }
}

/// The root group of each structure of `version`, refusing a second root of
/// a structure.
pub(super) fn roots<'a>(
    version: &'a VersionTables,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<BTreeMap<String, &'a GroupRow>, LegacyError> {
    let mut out = BTreeMap::new();
    for group in version.groups.iter().filter(|group| group.is_root) {
        if out.contains_key(&group.message_id) {
            let listed = version
                .messages
                .iter()
                .any(|message| message.id == group.message_id);
            if listed {
                return Err(invalid(
                    &version.file("groups"),
                    &group.name,
                    "a second root group of its structure",
                ));
            }
            tolerate(
                hits,
                &version.file("groups"),
                &group.name,
                LegacyDefect::UnlistedSecondRoot,
            )?;
            continue;
        }
        out.insert(group.message_id.clone(), group);
    }
    Ok(out)
}

// NOTE: the extract dates a withdrawal (its last version listing ORM_O01 is 2.6); v2.9.1 chapter 4
// §7.5.1 withdraws only its query example, and table 0354, whose `v2-table-deprecated` is empty,
// must mark the code `deprecated`.
fn check_table_0354(
    table: &Table0354,
    code: &str,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<(), LegacyError> {
    let location = format!("{TABLE_0354}#{code}");
    match table.status(code) {
        CodeStatus::Stated("deprecated") => Ok(()),
        CodeStatus::Stated("active") => {
            tolerate(hits, &location, code, LegacyDefect::ActiveInTable0354)
        }
        CodeStatus::Absent => tolerate(hits, &location, code, LegacyDefect::NotInTable0354),
        other @ (CodeStatus::Stated(_) | CodeStatus::Unstated) => Err(invalid(
            TABLE_0354,
            code,
            format!("table 0354 gives the status {other:?}"),
        )),
    }
}
