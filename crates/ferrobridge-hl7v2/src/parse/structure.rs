// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The selection of the message structure a lexed message is grouped by.

use hl7v2_types::model::Structure;

use crate::parse::{Message, base_name};

/// Why a structure could not be selected for a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StructureError {
    /// MSH-9.3 is not valued and the message index names no structure for
    /// MSH-9.1 and MSH-9.2.
    #[error(
        "MSH-9.3 names no message structure and none is defined for the message type and trigger event"
    )]
    Unnamed,
    /// MSH-9.3 names no structure of the definitions.
    #[error("MSH-9.3 names the unknown message structure {name:?}")]
    Unknown {
        /// MSH-9.3 as the message wrote it.
        name: String,
    },
    /// MSH-9.3 names a structure the definitions carry in several variants,
    /// and no message definition for MSH-9.1 and MSH-9.2 names one of them.
    #[error("the message structure {name:?} has the variants {candidates:?}")]
    Variant {
        /// MSH-9.3 as the message wrote it.
        name: String,
        /// The variant ids.
        candidates: Vec<&'static str>,
    },
    /// The message names a structure only the legacy tables carry, and none
    /// of them carries it at or before the version MSH-12 declares.
    #[error(
        "the withdrawn message structure {name:?} has no tree at or before the version {declared:?}; the legacy tables carry it at {versions:?}"
    )]
    NoLegacyVersion {
        /// The structure id, from MSH-9.3 or from MSH-9.1 and MSH-9.2.
        name: String,
        /// MSH-12.1 as the message wrote it.
        declared: Option<String>,
        /// The versions whose tables carry the structure.
        versions: Vec<&'static str>,
    },
}

/// Selects the message structure MSH-9.3 names.
///
/// A structure the definitions carry once (`ACK`, `ADT_A02`) is selected by
/// its id. One they carry in variants (`ORU_R01-A` to `ORU_R01-D`) is
/// selected by the message definition of MSH-9.1 and MSH-9.2
/// ([`hl7v2_types::message::find`]): `ORU^R01` names `ORU_R01-A` and
/// `ADT^A04` names `ADT_A01-B`. When MSH-9.3 names no structure of the
/// definitions (`ADT^A08^ADT_A08`), the message definition of MSH-9.1 and
/// MSH-9.2 selects it (`ADT_A01`), and [`group`] counts
/// [`Unplaced::OtherStructure`]. A structure the definitions no longer carry
/// (`ORM_O01`), which no message definition names, is selected from the
/// legacy tables ([`hl7v2_types::legacy::versions`], and
/// [`hl7v2_types::message::legacy`] when MSH-9.3 is empty) by the version
/// MSH-12 declares, or the nearest earlier version that carries it.
///
/// A structure the definitions carry is then taken from the tables of the
/// version MSH-12 declares ([`hl7v2_types::legacy::find`] by the id without
/// its variant, `ORU_R01` at `2.5.1`), so its tree and its segments' field
/// optionality are that version's. The v2.9.1 structure stays the answer for
/// a message declaring 2.9.1, a version the legacy tables do not carry (2.9,
/// a version after 2.8.2, or none), or one whose tables lack the structure;
/// [`group`] counts [`Unplaced::VersionSelected`] with the version used.
///
/// # Errors
///
/// Returns [`StructureError`] when MSH-9.3 is empty or names no structure
/// and neither message index names one, when MSH-9.3 names one with variants
/// of which the message definition names none, and when the legacy tables
/// carry the structure only after the version MSH-12 declares.
///
/// [`group`]: crate::parse::grouping::group
/// [`Unplaced::OtherStructure`]: crate::parse::Unplaced::OtherStructure
/// [`Unplaced::VersionSelected`]: crate::parse::Unplaced::VersionSelected
pub fn structure_for(message: &Message) -> Result<&'static Structure, StructureError> {
    let structure = definitions_structure(message)?;
    if structure.withdrawn_as_of.is_some() {
        return Ok(structure);
    }
    // NOTE: HL7 v2.9.1 MSH-12 (hl7-v2ig MSH.json `MSH.12-versionId`) is "matched by the receiving
    // system to its own version", so a version's own tables are used where carried; no vendored
    // text rules on a version they lack, so the v2.9.1 fallback is our own design.
    let own = message
        .version()
        .filter(|declared| *declared != crate::DEFINITIONS_VERSION)
        .and_then(|declared| hl7v2_types::legacy::find(base_name(structure.id), declared));
    Ok(own.unwrap_or(structure))
}

/// Selects the structure of the v2.9.1 definitions, or of the legacy tables
/// for one they no longer carry, by the rules of [`structure_for`].
fn definitions_structure(message: &Message) -> Result<&'static Structure, StructureError> {
    let code_event = message.message_type(1).zip(message.message_type(2));
    let indexed = code_event
        .and_then(|(code, event)| hl7v2_types::message::find(code, event))
        .and_then(|definition| definition.structure);
    let Some(name) = message.message_type(3) else {
        // NOTE: HL7 v2.5.1 chapter 2 §2.15.9.9 makes MSH-9.3 optional where table
        // 0354 fixes the structure, so the message indexes answer for a header without it.
        if let Some(structure) = indexed {
            return Ok(structure);
        }
        let Some((code, event)) = code_event else {
            return Err(StructureError::Unnamed);
        };
        let entries = hl7v2_types::message::legacy(code, event);
        let Some(first) = entries.first() else {
            return Err(StructureError::Unnamed);
        };
        let trees: Vec<&'static Structure> = entries.iter().map(|entry| entry.structure).collect();
        return legacy_tree(first.structure.id, &trees, message.version());
    };
    if let Some(structure) = hl7v2_types::structure::find(name) {
        return Ok(structure);
    }
    let prefix = format!("{name}-");
    let candidates: Vec<&'static str> = hl7v2_types::structure::STRUCTURES
        .iter()
        .map(|structure| structure.id)
        .filter(|id| id.starts_with(&prefix))
        .collect();
    if candidates.is_empty() {
        // NOTE: HL7 v2.5.1 chapter 2 §2.15.9.9: table 0354 fixes the structure from MSH-9.1
        // and MSH-9.2, so the index answers for an MSH-9.3 naming none, counted by `group`.
        if let Some(structure) = indexed {
            return Ok(structure);
        }
        let trees = hl7v2_types::legacy::versions(name);
        if trees.is_empty() {
            return Err(StructureError::Unknown {
                name: String::from(name),
            });
        }
        return legacy_tree(name, trees, message.version());
    }
    let named = indexed.filter(|structure| candidates.contains(&structure.id));
    named.ok_or_else(|| StructureError::Variant {
        name: String::from(name),
        candidates,
    })
}

/// Picks, from the legacy `trees` of one structure in version order, the
/// tree of the version `declared` names, or of the nearest earlier version.
fn legacy_tree(
    name: &str,
    trees: &[&'static Structure],
    declared: Option<&str>,
) -> Result<&'static Structure, StructureError> {
    // NOTE: no specification governs this: our own design; a version the legacy tables do not
    // carry is parsed against the nearest earlier version that does, and the outcome names both.
    let chosen = declared.and_then(version_key).and_then(|declared| {
        trees
            .iter()
            .copied()
            .rev()
            .find(|tree| version_key(tree.version).is_some_and(|key| key <= declared))
    });
    chosen.ok_or_else(|| StructureError::NoLegacyVersion {
        name: String::from(name),
        declared: declared.map(String::from),
        versions: trees.iter().map(|tree| tree.version).collect(),
    })
}

/// The numeric components of a dotted version (`2.5.1` is `[2, 5, 1]`);
/// `None` for any other text.
fn version_key(version: &str) -> Option<Vec<u32>> {
    // NOTE: a version that is not dotted digits names no legacy tree, so the
    // parse failure is the answer and the message is refused with the versions.
    version
        .split('.')
        .map(|part| {
            let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            if digits { part.parse().ok() } else { None }
        })
        .collect()
}
