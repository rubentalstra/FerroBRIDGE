// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering the legacy tables to the shapes of [`crate::v2::lower`].
//!
//! The root set is every message structure a version's `messages.json` lists
//! and `groups.json` gives a root, at every version of the extract. A
//! structure's tree is the elements under its root group, ordered by row id
//! among siblings and renumbered from 1, with element ids
//! derived by the rule of the v2.9.1 definitions (`ORM_O01.2-PATIENT.1-PID`,
//! `choice-n-NAME` under a choice). Each segment a tree names is lowered from
//! the same version's `segments.json`, `fields.json` and `data_elements.json`.
//! A structure whose code no structure of the v2.9.1 definitions carries
//! (variants aside: `ORU_R01-A` carries `ORU_R01`) is marked withdrawn as of
//! the first version, among the extract's versions and then v2.9.1, that
//! lists it no more, and table 0354 must mark its code deprecated.
//!
//! Nothing is emitted twice ([`Owner`]): a segment whose field table agrees
//! with the v2.9.1 segment links to that segment, one identical to a segment
//! an earlier version emits links to that one, and a tree identical to the
//! v2.9.1 tree of its id or to an earlier version's, with every segment it
//! places linked to the same static, links to that tree.
//!
//! Each defect of the export is tolerated only where it was found
//! ([`LegacyDefect::tolerated_in`]); the same defect anywhere else is refused.

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::legacy::datatype::{LegacyDataType, lower_data_types};
use crate::v2::legacy::source::{
    CodeStatus, DataElementRow, ElementRow, FieldRow, GroupRow, TABLE_0354, Table0354, Tables,
    VersionTables, version_key,
};
use crate::v2::lower::{
    Cardinality, DEFINITIONS_VERSION, Field, GroupKind, Length, Model, Node, Optionality, Segment,
    Structure, TABLE_VALUE_SET, Table,
};

/// A defect of the legacy tables that lowering tolerates where it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyDefect {
    /// Two siblings of a tree share an element position.
    RepeatedPosition,
    /// `messages.json` lists a structure that `groups.json` gives no root.
    MessageWithoutTree,
    /// A group of a structure that no element of its tree places.
    UnreachedGroup,
    /// `groups.json` gives a second root to a structure `messages.json` does
    /// not list.
    UnlistedSecondRoot,
    /// A structure's root group has no elements; the version emits no tree
    /// for it.
    EmptyRoot,
    /// An element of a structure places a group of another; the version
    /// emits no tree for the structure.
    ForeignGroup,
    /// A tree places MSH other than as the first element of its root; the
    /// version emits no tree for the structure.
    SecondHeader,
    /// A data element writes its maximum length `.`.
    DotLength,
    /// A segment's field rows skip a position.
    MissingField,
    /// A field marked `R` where the versions before and after it and v2.9.1
    /// mark it `C`; it is emitted `C`.
    StrayRequired,
    /// A tree places the slot `Zxx`.
    ZxxSlot,
    /// A segment a tree places has no row in `fields.json`.
    SegmentWithoutFields,
    /// A choice group holds an optional member.
    OptionalChoiceMember,
    /// Table 0354 has no concept for a structure code the v2.9.1 definitions lack.
    NotInTable0354,
    /// Table 0354 marks `active` a structure code the v2.9.1 definitions lack.
    ActiveInTable0354,
}

impl LegacyDefect {
    /// The files where the tables carry the defect, relative to the tables
    /// directory, or the table 0354 concepts as `<file>#<code>`.
    #[must_use]
    pub const fn tolerated_in(self) -> &'static [&'static str] {
        match self {
            // NOTE: no specification governs this: our own design; these files repeat a position
            // among siblings (from 2.5 on often 1 per segment, 2 per group), and the row ids follow
            // the order positions give wherever those are distinct, so siblings order by row id.
            Self::RepeatedPosition => &[
                "2.3/elements.json",
                "2.3.1/elements.json",
                "2.4/elements.json",
                "2.5/elements.json",
                "2.5.1/elements.json",
                "2.6/elements.json",
                "2.7/elements.json",
                "2.7.1/elements.json",
                "2.8/elements.json",
                "2.8.1/elements.json",
                "2.8.2/elements.json",
            ],
            // NOTE: these files list structures with no root group (2.1 and 2.2 list only ACK and
            // carry no groups, 2.3 has 10 trees of 114); each is taken from the versions that have one.
            Self::MessageWithoutTree => &[
                "2.1/messages.json",
                "2.2/messages.json",
                "2.3/messages.json",
                "2.3.1/messages.json",
                "2.4/messages.json",
            ],
            // NOTE: these files carry a second group row of one name that no element places
            // (ORM_O01.ORDER at 2.3); the tree is the one its root reaches.
            Self::UnreachedGroup => &[
                "2.3/groups.json",
                "2.5/groups.json",
                "2.5.1/groups.json",
                "2.7/groups.json",
                "2.8/groups.json",
            ],
            // NOTE: this file gives QBP_Qnn, which its messages.json does not list, two root
            // groups; the structure is outside the root set and the first root is kept.
            Self::UnlistedSecondRoot => &["2.6/groups.json"],
            // NOTE: these files give the roots of the donation structures (DBC_O41 to DRG_O43,
            // QBP_O33, RSP_O34) no element and hang their elements under other roots.
            Self::EmptyRoot => &["2.8.1/groups.json", "2.8.2/groups.json"],
            // NOTE: these files hang the donation structures' elements under other roots (the
            // DBC_O41.DONOR group under CSU_C09.ROOT, a second MSH under QBP_E22.ROOT), leaving those trees unusable.
            Self::ForeignGroup | Self::SecondHeader => {
                &["2.8.1/elements.json", "2.8.2/elements.json"]
            }
            // NOTE: these files write the maximum length of item 00703 (Column Value, VARIES) as
            // `.`, read as no stated maximum.
            Self::DotLength => &["2.8.1/data_elements.json", "2.8.2/data_elements.json"],
            // NOTE: this file gives RF1 no row at position 18; the position is emitted untyped,
            // unnamed, 0..1 and with no optionality code, so no value there is refused.
            Self::MissingField => &["2.8.1/fields.json"],
            // NOTE: these files mark OBX-4 (Observation Sub-ID) `R` where 2.2 to 2.7, 2.8.1, 2.8.2
            // and the v2.9.1 OBX.json mark it `C`; it is emitted `C`, an export defect to report.
            Self::StrayRequired => &["2.7.1/fields.json#OBX.4", "2.8/fields.json#OBX.4"],
            // NOTE: MFN_M01.MF and MFR_M01.MF_QUERY place `Zxx` where 2.5 places `Hxx`,
            // so it is emitted as the same open slot.
            Self::ZxxSlot => &["2.3.1/elements.json", "2.4/elements.json"],
            // NOTE: these files give ED (2.5 to 2.6) or QRD, QRF, URD and URS (2.7 on)
            // no field rows; the segment is emitted with an empty field table.
            Self::SegmentWithoutFields => &[
                "2.5/fields.json",
                "2.5.1/fields.json",
                "2.6/fields.json",
                "2.7/fields.json",
                "2.7.1/fields.json",
                "2.8/fields.json",
                "2.8.1/fields.json",
                "2.8.2/fields.json",
            ],
            // NOTE: these files nest optional segments inside a choice group (NTE under
            // ORR_O02.CHOICE at 2.3.1); the tree is emitted as written.
            Self::OptionalChoiceMember => &[
                "2.3.1/elements.json",
                "2.4/elements.json",
                "2.5/elements.json",
                "2.5.1/elements.json",
                "2.6/elements.json",
                "2.7/elements.json",
                "2.7.1/elements.json",
                "2.8/elements.json",
                "2.8.1/elements.json",
                "2.8.2/elements.json",
            ],
            // NOTE: CodeSystem-v2-0354.json 3.0.0 has no concept for these codes the
            // extract carries and the v2.9.1 definitions lack.
            Self::NotInTable0354 => &[
                "hl7.terminology/package/CodeSystem-v2-0354.json#QBP_K13",
                "hl7.terminology/package/CodeSystem-v2-0354.json#RSP_K13",
                "hl7.terminology/package/CodeSystem-v2-0354.json#RSP_K15",
                "hl7.terminology/package/CodeSystem-v2-0354.json#RTB_Q13",
            ],
            // NOTE: CodeSystem-v2-0354.json 3.0.0 marks these codes active although
            // the v2.9.1 definitions carry no structure for them.
            Self::ActiveInTable0354 => &[
                "hl7.terminology/package/CodeSystem-v2-0354.json#RDR_RDR",
                "hl7.terminology/package/CodeSystem-v2-0354.json#RSP_K24",
                "hl7.terminology/package/CodeSystem-v2-0354.json#UDM_Q05",
            ],
        }
    }

    /// Whether the tables carry the defect at `location`.
    #[must_use]
    pub fn is_tolerated(self, location: &str) -> bool {
        self.tolerated_in().contains(&location)
    }
}

/// A failure while lowering the legacy tables.
#[derive(Debug, thiserror::Error)]
pub enum LegacyError {
    /// A row breaks a rule the lowering relies on.
    #[error("{file}: {row}: {reason}")]
    Invalid {
        /// The file, relative to the tables directory.
        file: String,
        /// The row: a structure, group, element or field.
        row: String,
        /// What is wrong.
        reason: String,
    },
    /// A defect found outside the places where it is tolerated.
    #[error("{location}: {row}: {defect:?} outside the files that carry it")]
    Defect {
        /// The file, or the table 0354 concept.
        location: String,
        /// The row.
        row: String,
        /// The defect.
        defect: LegacyDefect,
    },
    /// A count of the tables does not fit the emitted integer type.
    #[error("{file}: {row}: a count beyond the emitted integer type")]
    Range {
        /// The file, relative to the tables directory.
        file: String,
        /// The row.
        row: String,
        /// Why it does not fit.
        #[source]
        source: std::num::TryFromIntError,
    },
    /// A number of the tables does not parse.
    #[error("{file}: {row}: {value:?} is not a count the tables hold")]
    Number {
        /// The file, relative to the tables directory.
        file: String,
        /// The row.
        row: String,
        /// The value as written.
        value: String,
        /// Why it does not parse.
        #[source]
        source: std::num::ParseIntError,
    },
}

/// Where a static that a version links to lives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    /// The v2.9.1 definitions.
    Current,
    /// The tables of the named earlier version.
    Version(String),
}

/// One version's structures and the segments they name.
#[derive(Debug, Clone)]
pub struct LegacyVersion {
    /// The version, for example `2.5.1`.
    pub version: String,
    /// Every structure of the root set this version carries, by id.
    pub structures: BTreeMap<String, Structure>,
    /// The structures whose tree is identical to the tree of an earlier
    /// static, by id, with where that tree lives.
    pub trees: BTreeMap<String, Owner>,
    /// The segments the trees name that no earlier static carries, by id.
    pub segments: BTreeMap<String, Segment>,
    /// The segments the trees name that link to an earlier static, by id:
    /// the v2.9.1 segment where the field tables agree, and otherwise the
    /// first version that emits the identical segment.
    pub links: BTreeMap<String, Owner>,
    /// The data type codes the fields of `segments` name, by code.
    pub data_types: BTreeMap<String, LegacyDataType>,
}

impl LegacyVersion {
    /// Where the static of the segment `id` this version's trees name lives.
    #[must_use]
    pub fn segment_owner(&self, id: &str) -> Owner {
        self.links
            .get(id)
            .cloned()
            .unwrap_or_else(|| Owner::Version(self.version.clone()))
    }
}

/// One entry of the legacy message index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMessage {
    /// The message code.
    pub code: String,
    /// The trigger event.
    pub event: String,
    /// The version.
    pub version: String,
    /// The structure id.
    pub structure: String,
}

/// The lowered legacy tables.
#[derive(Debug, Clone)]
pub struct LegacyModel {
    /// The pinned commit of the export.
    pub commit: String,
    /// Every version carrying a structure of the root set, in version order.
    pub versions: Vec<LegacyVersion>,
    /// The message index in `(code, event, version)` order, versions ordered
    /// numerically.
    pub messages: Vec<LegacyMessage>,
    /// Every tolerated defect the lowering met, with where.
    pub tolerated: BTreeSet<(LegacyDefect, String)>,
}

impl LegacyModel {
    /// Lowers the root set of `tables` against the v2.9.1 model `current`,
    /// checking each code against `table`.
    ///
    /// # Errors
    ///
    /// Returns [`LegacyError::Invalid`] naming the file and row that break a
    /// rule the lowering relies on, [`LegacyError::Number`] for a count that
    /// does not parse, and [`LegacyError::Defect`] for a known defect outside
    /// the places where it is tolerated, a table 0354 disagreement included.
    pub fn lower(tables: &Tables, current: &Model, table: &Table0354) -> Result<Self, LegacyError> {
        let mut hits = BTreeSet::new();
        let current_codes: BTreeSet<&str> =
            current.structures.keys().map(|id| base_code(id)).collect();
        let (listed, rooted) = listing(tables, &current_codes, &mut hits)?;
        let withdrawn = withdrawals(tables, &listed, &rooted, table, &mut hits)?;
        let mut versions: Vec<LegacyVersion> = Vec::new();
        let mut messages = Vec::new();
        for (index, version) in tables.versions().iter().enumerate() {
            let Some(roots) = rooted.get(index) else {
                continue;
            };
            let mut lowering = Lowering::new(version, &mut hits);
            let mut structures = BTreeMap::new();
            for message in &version.messages {
                let Some(root) = roots.get(message.id.as_str()) else {
                    continue;
                };
                if let Some((defect, file)) = lowering.damage(&message.id, root) {
                    tolerate(lowering.hits, &version.file(file), &message.id, defect)?;
                    continue;
                }
                let withdrawn_as_of = if current_codes.contains(message.id.as_str()) {
                    None
                } else {
                    let Some(withdrawn_as_of) = withdrawn.get(&message.id) else {
                        return Err(invalid(
                            &version.file("messages"),
                            &message.id,
                            "a rooted structure with no version it is withdrawn as of",
                        ));
                    };
                    Some(withdrawn_as_of.as_str())
                };
                let structure = lowering.structure(&message.id, root, withdrawn_as_of)?;
                structures.insert(message.id.clone(), structure);
                messages.push(LegacyMessage {
                    code: message.msg_type_id.clone(),
                    event: message.event_id.clone(),
                    version: version.version.clone(),
                    structure: message.id.clone(),
                });
            }
            if structures.is_empty() {
                continue;
            }
            let (segments, links) = lowering.segments(current, &versions)?;
            let data_types = lower_data_types(version, &segments, &current.data_types)?;
            let mut lowered = LegacyVersion {
                version: version.version.clone(),
                structures,
                trees: BTreeMap::new(),
                segments,
                links,
                data_types,
            };
            let trees = lowered
                .structures
                .values()
                .filter_map(|structure| {
                    tree_owner(&lowered, structure, current, &versions)
                        .map(|owner| (structure.id.clone(), owner))
                })
                .collect();
            lowered.trees = trees;
            versions.push(lowered);
        }
        messages.sort_by(|left, right| {
            (&left.code, &left.event, version_key(&left.version)).cmp(&(
                &right.code,
                &right.event,
                version_key(&right.version),
            ))
        });
        Ok(Self {
            commit: tables.commit().to_owned(),
            versions,
            messages,
            tolerated: hits,
        })
    }

    /// The number of structures over every version.
    #[must_use]
    pub fn structure_count(&self) -> usize {
        self.versions.iter().map(|v| v.structures.len()).sum()
    }

    /// The number of distinct structure codes.
    #[must_use]
    pub fn code_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.structures.keys())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// The number of structures, over every version, whose code the v2.9.1
    /// definitions lack.
    #[must_use]
    pub fn withdrawn_count(&self) -> usize {
        self.withdrawn().count()
    }

    /// The number of distinct structure codes the v2.9.1 definitions lack.
    #[must_use]
    pub fn withdrawn_code_count(&self) -> usize {
        self.withdrawn()
            .map(|structure| &structure.id)
            .collect::<BTreeSet<_>>()
            .len()
    }

    fn withdrawn(&self) -> impl Iterator<Item = &Structure> {
        self.versions
            .iter()
            .flat_map(|v| v.structures.values())
            .filter(|structure| structure.withdrawn_as_of.is_some())
    }

    /// The number of trees emitted over every version: the structures whose
    /// tree links to no other static.
    #[must_use]
    pub fn tree_count(&self) -> usize {
        self.structure_count()
            .saturating_sub(self.versions.iter().map(|v| v.trees.len()).sum())
    }

    /// The number of structures whose tree links to the v2.9.1 tree of its id.
    #[must_use]
    pub fn current_tree_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.trees.values())
            .filter(|owner| **owner == Owner::Current)
            .count()
    }

    /// The number of structures whose tree links to an earlier version's
    /// identical tree.
    #[must_use]
    pub fn inherited_tree_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.trees.values())
            .filter(|owner| matches!(owner, Owner::Version(_)))
            .count()
    }

    /// The number of segments emitted over every version.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.versions.iter().map(|v| v.segments.len()).sum()
    }

    /// The number of segment references linked to a v2.9.1 segment.
    #[must_use]
    pub fn shared_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.links.values())
            .filter(|owner| **owner == Owner::Current)
            .count()
    }

    /// The number of segment references linked to an earlier version's
    /// identical segment.
    #[must_use]
    pub fn inherited_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.links.values())
            .filter(|owner| matches!(owner, Owner::Version(_)))
            .count()
    }

    /// The number of fields over every emitted segment.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.segments.values())
            .map(|segment| segment.fields.len())
            .sum()
    }
}

/// Returns where the tree of `structure`, a structure of `version`, already
/// lives, or `None` when `version` emits the tree itself.
///
/// The owner is the v2.9.1 tree of its id when the nodes are equal and every
/// segment it places links to v2.9.1, and otherwise the first tree of an
/// `earlier` version with equal nodes whose placed segments resolve to the
/// same statics.
#[must_use]
pub fn tree_owner(
    version: &LegacyVersion,
    structure: &Structure,
    current: &Model,
    earlier: &[LegacyVersion],
) -> Option<Owner> {
    let mut placed = BTreeSet::new();
    placed_segments(&structure.nodes, &mut placed);
    let defined = current
        .structures
        .get(&structure.id)
        .is_some_and(|defined| defined.nodes == structure.nodes);
    if defined
        && placed
            .iter()
            .all(|id| version.segment_owner(id) == Owner::Current)
    {
        return Some(Owner::Current);
    }
    earlier.iter().find_map(|other| {
        let tree = other
            .structures
            .get(&structure.id)
            .filter(|_| !other.trees.contains_key(&structure.id))?;
        let same = tree.nodes == structure.nodes
            && placed
                .iter()
                .all(|id| other.segment_owner(id) == version.segment_owner(id));
        same.then(|| Owner::Version(other.version.clone()))
    })
}

/// Every segment id `nodes` place, at any depth.
fn placed_segments<'n>(nodes: &'n [Node], out: &mut BTreeSet<&'n str>) {
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

fn listing<'t>(
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
fn withdrawals(
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
fn base_code(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((base, variant)) if variant.len() == 1 => base,
        _ => id,
    }
}

fn invalid(file: &str, row: &str, reason: impl Into<String>) -> LegacyError {
    LegacyError::Invalid {
        file: file.to_owned(),
        row: row.to_owned(),
        reason: reason.into(),
    }
}

fn tolerate(
    hits: &mut BTreeSet<(LegacyDefect, String)>,
    location: &str,
    row: &str,
    defect: LegacyDefect,
) -> Result<(), LegacyError> {
    if defect.is_tolerated(location) {
        hits.insert((defect, location.to_owned()));
        Ok(())
    } else {
        Err(LegacyError::Defect {
            location: location.to_owned(),
            row: row.to_owned(),
            defect,
        })
    }
}

/// The root group of each structure of `version`, refusing a second root of
/// a structure.
fn roots<'a>(
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

/// The segments a version emits, and the static each other segment it names
/// links to.
type SegmentLinks = (BTreeMap<String, Segment>, BTreeMap<String, Owner>);

/// The lowering of one version: its trees, and the segments they reach.
struct Lowering<'a, 'h> {
    version: &'a VersionTables,
    hits: &'h mut BTreeSet<(LegacyDefect, String)>,
    groups: BTreeMap<u64, &'a GroupRow>,
    children: BTreeMap<u64, Vec<&'a ElementRow>>,
    /// The structures one of whose elements places a group of another.
    hosts: BTreeSet<&'a str>,
    reached: BTreeSet<String>,
}

impl<'a, 'h> Lowering<'a, 'h> {
    fn new(version: &'a VersionTables, hits: &'h mut BTreeSet<(LegacyDefect, String)>) -> Self {
        let groups: BTreeMap<u64, &GroupRow> = version
            .groups
            .iter()
            .map(|group| (group.id, group))
            .collect();
        let mut children: BTreeMap<u64, Vec<&ElementRow>> = BTreeMap::new();
        let mut hosts = BTreeSet::new();
        for element in &version.elements {
            children.entry(element.parent_id).or_default().push(element);
            let placed = element.group_id.and_then(|child| groups.get(&child));
            let parent = groups.get(&element.parent_id);
            if let (Some(placed), Some(parent)) = (placed, parent)
                && placed.message_id != parent.message_id
            {
                hosts.insert(parent.message_id.as_str());
            }
        }
        for siblings in children.values_mut() {
            siblings.sort_by_key(|element| element.id);
        }
        Self {
            version,
            hits,
            groups,
            children,
            hosts,
            reached: BTreeSet::new(),
        }
    }

    /// The defect that keeps the tree of the structure `id` under `root` out
    /// of this version, with the file that carries it, if any: a root with no
    /// element, a group of another structure in it, or an MSH anywhere but
    /// first.
    fn damage(&self, id: &str, root: &GroupRow) -> Option<(LegacyDefect, &'static str)> {
        if !self.children.contains_key(&root.id) {
            Some((LegacyDefect::EmptyRoot, "groups"))
        } else if self.hosts.contains(id) {
            Some((LegacyDefect::ForeignGroup, "elements"))
        } else if self.second_header(root.id, &mut BTreeSet::new()) {
            Some((LegacyDefect::SecondHeader, "elements"))
        } else {
            None
        }
    }

    /// Whether the elements under the group `id` place an MSH other than the
    /// first element of the root, the first group `seen` holding the root.
    fn second_header(&self, id: u64, seen: &mut BTreeSet<u64>) -> bool {
        let root = seen.is_empty();
        if !seen.insert(id) {
            return false;
        }
        let Some(elements) = self.children.get(&id) else {
            return false;
        };
        elements.iter().enumerate().any(|(index, element)| {
            let header = element.segment_id.as_deref() == Some("MSH") && !(root && index == 0);
            header
                || element
                    .group_id
                    .is_some_and(|child| self.second_header(child, seen))
        })
    }

    fn structure(
        &mut self,
        id: &str,
        root: &GroupRow,
        withdrawn_as_of: Option<&str>,
    ) -> Result<Structure, LegacyError> {
        if root.is_choice {
            return Err(invalid(
                &self.version.file("groups"),
                &root.name,
                "the root is a choice",
            ));
        }
        let mut seen = BTreeSet::from([root.id]);
        let nodes = self.children_of(id, root, id, &mut seen)?;
        for group in self
            .version
            .groups
            .iter()
            .filter(|group| group.message_id == id)
        {
            if !seen.contains(&group.id) {
                tolerate(
                    self.hits,
                    &self.version.file("groups"),
                    &group.name,
                    LegacyDefect::UnreachedGroup,
                )?;
            }
        }
        Ok(Structure {
            id: id.to_owned(),
            url: None,
            version: self.version.version.clone(),
            withdrawn_as_of: withdrawn_as_of.map(str::to_owned),
            nodes,
        })
    }

    fn children_of(
        &mut self,
        structure: &str,
        group: &GroupRow,
        path: &str,
        seen: &mut BTreeSet<u64>,
    ) -> Result<Vec<Node>, LegacyError> {
        let elements_file = self.version.file("elements");
        let Some(elements) = self.children.get(&group.id).cloned() else {
            return Err(invalid(
                &elements_file,
                &group.name,
                "a group with no children",
            ));
        };
        let mut nodes = Vec::with_capacity(elements.len());
        let mut positions = BTreeSet::new();
        for element in &elements {
            let row = element.id.to_string();
            let repeated = !positions.insert(element.position);
            self.check_element(group, element, repeated)?;
            let position = u16::try_from(nodes.len().saturating_add(1)).map_err(|source| {
                LegacyError::Range {
                    file: elements_file.clone(),
                    row: row.clone(),
                    source,
                }
            })?;
            let cardinality = cardinality(&elements_file, &row, element.min, &element.max)?;
            let step = |name: &str| {
                if group.is_choice {
                    format!("{path}.choice-{position}-{name}")
                } else {
                    format!("{path}.{position}-{name}")
                }
            };
            let node = match (&element.segment_id, element.group_id) {
                (Some(segment), None) => match segment.as_str() {
                    "Hxx" | "Zxx" => {
                        if segment == "Zxx" {
                            tolerate(self.hits, &elements_file, &row, LegacyDefect::ZxxSlot)?;
                        }
                        Node::Placeholder {
                            id: step(segment),
                            position,
                            cardinality,
                        }
                    }
                    _ => {
                        self.reached.insert(segment.clone());
                        Node::Segment {
                            id: step(segment),
                            position,
                            segment: segment.clone(),
                            cardinality,
                            status: None,
                        }
                    }
                },
                (None, Some(child)) => {
                    let (inner, name) = self.placed_group(structure, child, &row, seen)?;
                    let id = step(name);
                    let children = self.children_of(structure, inner, &id, seen)?;
                    Node::Group {
                        id,
                        position,
                        name: name.to_owned(),
                        cardinality,
                        kind: if inner.is_choice {
                            GroupKind::Choice
                        } else {
                            GroupKind::Sequence
                        },
                        children,
                    }
                }
                _ => {
                    return Err(invalid(
                        &elements_file,
                        &row,
                        "not exactly one of a segment and a group",
                    ));
                }
            };
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Checks one element of `group`: whether an earlier sibling holds its
    /// position (`repeated`), its usage against its minimum, and a choice
    /// member's usage.
    fn check_element(
        &mut self,
        group: &GroupRow,
        element: &ElementRow,
        repeated: bool,
    ) -> Result<(), LegacyError> {
        let file = self.version.file("elements");
        let row = element.id.to_string();
        if repeated {
            tolerate(self.hits, &file, &row, LegacyDefect::RepeatedPosition)?;
        }
        let required = element.usage == "R";
        if !(required || element.usage == "O") || required != (element.min > 0) {
            return Err(invalid(
                &file,
                &row,
                format!("usage {:?} with min {}", element.usage, element.min),
            ));
        }
        if group.is_choice && !required {
            tolerate(self.hits, &file, &row, LegacyDefect::OptionalChoiceMember)?;
        }
        Ok(())
    }

    /// The group row `child` an element of `structure` places, with its name,
    /// refusing one outside the structure, a root, or one placed twice.
    fn placed_group(
        &self,
        structure: &str,
        child: u64,
        row: &str,
        seen: &mut BTreeSet<u64>,
    ) -> Result<(&'a GroupRow, &'a str), LegacyError> {
        let groups_file = self.version.file("groups");
        let Some(inner) = self.groups.get(&child).copied() else {
            return Err(invalid(
                &self.version.file("elements"),
                row,
                format!("group {child} is not a row"),
            ));
        };
        if inner.message_id != structure || inner.is_root || !seen.insert(inner.id) {
            return Err(invalid(
                &groups_file,
                &inner.name,
                "a group placed outside its structure, as a root, or twice",
            ));
        }
        let Some(name) = inner
            .name
            .strip_prefix(structure)
            .and_then(|rest| rest.strip_prefix('.'))
            .filter(|name| !name.is_empty() && !name.contains('.'))
        else {
            return Err(invalid(
                &groups_file,
                &inner.name,
                "not named <structure>.<group>",
            ));
        };
        Ok((inner, name))
    }

    /// The segments the trees reached that no earlier static carries, and
    /// the static each other one links to: the v2.9.1 segment of its id
    /// where the field tables agree, else the identical segment an `earlier`
    /// version emits.
    fn segments(
        self,
        current: &Model,
        earlier: &[LegacyVersion],
    ) -> Result<SegmentLinks, LegacyError> {
        let version = self.version;
        let elements: BTreeMap<&str, &DataElementRow> = version
            .data_elements
            .iter()
            .map(|element| (element.id.as_str(), element))
            .collect();
        let mut owned = BTreeMap::new();
        let mut links = BTreeMap::new();
        for id in &self.reached {
            let segment = lower_segment(version, id, &elements, self.hits)?;
            if current
                .segments
                .get(id)
                .is_some_and(|defined| same_table(&segment, defined))
            {
                links.insert(id.clone(), Owner::Current);
                continue;
            }
            let alone = BTreeMap::from([(id.clone(), segment.clone())]);
            let types = lower_data_types(version, &alone, &current.data_types)?;
            // NOTE: no specification governs this: our own design; a segment links to an earlier
            // version's only when its field table and every data type it names agree there too.
            let same = |other: &&LegacyVersion| {
                other.segments.get(id) == Some(&segment)
                    && types
                        .iter()
                        .all(|(code, data_type)| other.data_types.get(code) == Some(data_type))
            };
            if let Some(other) = earlier.iter().find(same) {
                links.insert(id.clone(), Owner::Version(other.version.clone()));
            } else {
                owned.insert(id.clone(), segment);
            }
        }
        Ok((owned, links))
    }
}

fn number(file: &str, row: &str, text: &str) -> Result<u32, LegacyError> {
    text.parse::<u32>().map_err(|source| LegacyError::Number {
        file: file.to_owned(),
        row: row.to_owned(),
        value: text.to_owned(),
        source,
    })
}

fn cardinality(file: &str, row: &str, min: u32, max: &str) -> Result<Cardinality, LegacyError> {
    let max = if max == "*" {
        None
    } else {
        Some(number(file, row, max)?)
    };
    if max.is_some_and(|max| min > max) {
        return Err(invalid(file, row, format!("min {min} above max {max:?}")));
    }
    Ok(Cardinality { min, max })
}

fn lower_segment(
    version: &VersionTables,
    id: &str,
    elements: &BTreeMap<&str, &DataElementRow>,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<Segment, LegacyError> {
    let fields_file = version.file("fields");
    let Some(row) = version.segments.iter().find(|segment| segment.id == id) else {
        return Err(invalid(
            &version.file("segments"),
            id,
            "a tree names a segment with no row",
        ));
    };
    let mut rows: Vec<(u16, &FieldRow)> = Vec::new();
    for field in version.fields.iter().filter(|field| field.segment_id == id) {
        let label = format!("{id}.{}", field.position);
        let position = number(&fields_file, &label, &field.position)?;
        let position = u16::try_from(position).map_err(|source| LegacyError::Range {
            file: fields_file.clone(),
            row: label.clone(),
            source,
        })?;
        rows.push((position, field));
    }
    rows.sort_by_key(|(position, _)| *position);
    if rows.is_empty() {
        tolerate(hits, &fields_file, id, LegacyDefect::SegmentWithoutFields)?;
    }
    let mut fields: Vec<Field> = Vec::with_capacity(rows.len());
    for (position, field) in rows {
        let label = format!("{id}.{position}");
        while usize::from(position) > fields.len().saturating_add(1) {
            let missing = u16::try_from(fields.len().saturating_add(1)).map_err(|source| {
                LegacyError::Range {
                    file: fields_file.clone(),
                    row: label.clone(),
                    source,
                }
            })?;
            tolerate(hits, &fields_file, &label, LegacyDefect::MissingField)?;
            fields.push(Field {
                id: format!("{id}.{missing}"),
                position: missing,
                name: String::new(),
                data_type: None,
                cardinality: Cardinality {
                    min: 0,
                    max: Some(1),
                },
                optionality: Optionality::Unstated,
                length: None,
                conformance_length: None,
                table: None,
                standards_status: None,
            });
        }
        let index = fields.len();
        if usize::from(position) != index.saturating_add(1) {
            return Err(invalid(
                &fields_file,
                &label,
                format!(
                    "position {position} where {} belongs",
                    index.saturating_add(1)
                ),
            ));
        }
        let Some(element) = elements.get(field.data_element_id.as_str()) else {
            return Err(invalid(
                &fields_file,
                &label,
                format!("data element {} has no row", field.data_element_id),
            ));
        };
        if element.max_length == "." {
            tolerate(
                hits,
                &version.file("data_elements"),
                &element.id,
                LegacyDefect::DotLength,
            )?;
        }
        let lowered = lower_field(version, label, position, field, element)?;
        fields.push(stray_required(version, lowered, hits)?);
    }
    Ok(Segment {
        id: id.to_owned(),
        url: None,
        name: row.description.clone(),
        fields,
    })
}

/// The field as emitted: `C` where [`LegacyDefect::StrayRequired`] lists the
/// field's `R` as an export defect, refusing a listed field that is not `R`.
fn stray_required(
    version: &VersionTables,
    mut field: Field,
    hits: &mut BTreeSet<(LegacyDefect, String)>,
) -> Result<Field, LegacyError> {
    let fields_file = version.file("fields");
    let stray = format!("{fields_file}#{}", field.id);
    if !LegacyDefect::StrayRequired
        .tolerated_in()
        .contains(&stray.as_str())
    {
        return Ok(field);
    }
    if field.optionality != Optionality::R {
        return Err(invalid(
            &fields_file,
            &field.id,
            "listed as a stray R but not R",
        ));
    }
    tolerate(hits, &stray, &field.id, LegacyDefect::StrayRequired)?;
    field.optionality = Optionality::C;
    Ok(field)
}

/// One field of a legacy segment, from its `fields.json` row and the data
/// element that row names.
fn lower_field(
    version: &VersionTables,
    label: String,
    position: u16,
    field: &FieldRow,
    element: &DataElementRow,
) -> Result<Field, LegacyError> {
    let fields_file = version.file("fields");
    let data_elements_file = version.file("data_elements");
    let min = number(&fields_file, &label, &field.min)?;
    let cardinality = cardinality(&fields_file, &label, min, &field.max)?;
    let optionality = match field.usage.as_str() {
        "R" => Optionality::R,
        "O" => Optionality::O,
        "C" => Optionality::C,
        "X" => Optionality::X,
        "B" => Optionality::B,
        "W" => Optionality::W,
        "NA" => Optionality::Na,
        other => {
            return Err(invalid(&fields_file, &label, format!("usage {other:?}")));
        }
    };
    let data_type = match element.datatype_id.as_str() {
        "-" => None,
        "" => return Err(invalid(&data_elements_file, &element.id, "no data type")),
        code => Some(code.to_owned()),
    };
    let length = match element.max_length.as_str() {
        "" | "." if element.min_length == 0 => None,
        "" | "." => Some(Length {
            min: element.min_length,
            max: None,
        }),
        text => Some(Length {
            min: element.min_length,
            max: Some(number(&data_elements_file, &element.id, text)?),
        }),
    };
    let table = element.table_id.as_ref().map(|table| Table {
        id: table.clone(),
        value_set: format!("{TABLE_VALUE_SET}{table}"),
    });
    Ok(Field {
        id: label,
        position,
        name: element.description.clone(),
        data_type,
        cardinality,
        optionality,
        length,
        conformance_length: None,
        table,
        standards_status: None,
    })
}

/// Whether a legacy segment's fields agree with a v2.9.1 segment's in
/// everything a parser reads: the count and, per position, the data type
/// code, the cardinality, the optionality and the table.
fn same_table(legacy: &Segment, defined: &Segment) -> bool {
    legacy.fields.len() == defined.fields.len()
        && legacy
            .fields
            .iter()
            .zip(&defined.fields)
            .all(|(left, right)| {
                left.position == right.position
                    && left.data_type == right.data_type
                    && left.cardinality == right.cardinality
                    && left.optionality == right.optionality
                    && left.table.as_ref().map(|table| &table.id)
                        == right.table.as_ref().map(|table| &table.id)
            })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{LegacyVersion, Owner, tree_owner};
    use crate::v2::lower::{Cardinality, Model, Node, Structure};

    fn tree(id: &str, version: &str, segments: &[&str]) -> Structure {
        let nodes = segments
            .iter()
            .zip(1_u16..)
            .map(|(segment, position)| Node::Segment {
                id: format!("{id}.{position}-{segment}"),
                position,
                segment: (*segment).to_owned(),
                cardinality: Cardinality {
                    min: 1,
                    max: Some(1),
                },
                status: None,
            })
            .collect();
        Structure {
            id: id.to_owned(),
            url: None,
            version: version.to_owned(),
            withdrawn_as_of: None,
            nodes,
        }
    }

    fn version(name: &str, structure: Structure, links: &[(&str, Owner)]) -> LegacyVersion {
        LegacyVersion {
            version: name.to_owned(),
            structures: BTreeMap::from([(structure.id.clone(), structure)]),
            trees: BTreeMap::new(),
            segments: BTreeMap::new(),
            links: links
                .iter()
                .map(|(id, owner)| ((*id).to_owned(), owner.clone()))
                .collect(),
            data_types: BTreeMap::new(),
        }
    }

    fn model(structures: Vec<Structure>) -> Model {
        Model {
            commit: String::new(),
            structures: structures
                .into_iter()
                .map(|structure| (structure.id.clone(), structure))
                .collect(),
            segments: BTreeMap::new(),
            data_types: BTreeMap::new(),
            messages: BTreeMap::new(),
            tolerated: BTreeSet::new(),
        }
    }

    #[test]
    fn two_identical_trees_share_the_earlier_static() {
        let earlier = version(
            "2.5",
            tree("ACK", "2.5", &["MSH", "MSA"]),
            &[("MSH", Owner::Version(String::from("2.4")))],
        );
        let later = version(
            "2.5.1",
            tree("ACK", "2.5.1", &["MSH", "MSA"]),
            &[
                ("MSH", Owner::Version(String::from("2.4"))),
                ("MSA", Owner::Version(String::from("2.5"))),
            ],
        );
        let structure = &later.structures["ACK"];
        assert_eq!(
            tree_owner(&later, structure, &model(Vec::new()), &[earlier]),
            Some(Owner::Version(String::from("2.5")))
        );
    }

    #[test]
    fn a_tree_placing_a_segment_through_another_static_keeps_its_own() {
        let earlier = version("2.5", tree("ACK", "2.5", &["MSH", "MSA"]), &[]);
        let later = version(
            "2.5.1",
            tree("ACK", "2.5.1", &["MSH", "MSA"]),
            &[("MSH", Owner::Version(String::from("2.5")))],
        );
        let structure = &later.structures["ACK"];
        assert_eq!(
            tree_owner(&later, structure, &model(Vec::new()), &[earlier]),
            None,
            "MSA is the 2.5.1 segment in the later tree and the 2.5 one in the earlier"
        );
    }

    #[test]
    fn a_tree_linked_to_an_earlier_one_is_not_an_owner() {
        let mut middle = version("2.5", tree("ACK", "2.5", &["MSH"]), &[]);
        middle
            .trees
            .insert(String::from("ACK"), Owner::Version(String::from("2.4")));
        let later = version(
            "2.5.1",
            tree("ACK", "2.5.1", &["MSH"]),
            &[("MSH", Owner::Version(String::from("2.5")))],
        );
        let structure = &later.structures["ACK"];
        assert_eq!(
            tree_owner(&later, structure, &model(Vec::new()), &[middle]),
            None
        );
    }

    #[test]
    fn a_tree_identical_to_the_v2_9_1_one_links_to_it() {
        let current = model(vec![tree("ACK", "2.9.1", &["MSH", "MSA"])]);
        let linked = version(
            "2.8.2",
            tree("ACK", "2.8.2", &["MSH", "MSA"]),
            &[("MSH", Owner::Current), ("MSA", Owner::Current)],
        );
        let structure = &linked.structures["ACK"];
        assert_eq!(
            tree_owner(&linked, structure, &current, &[]),
            Some(Owner::Current)
        );
        let own = version(
            "2.8.2",
            tree("ACK", "2.8.2", &["MSH", "MSA"]),
            &[("MSH", Owner::Current)],
        );
        let structure = &own.structures["ACK"];
        assert_eq!(tree_owner(&own, structure, &current, &[]), None);
    }
}
