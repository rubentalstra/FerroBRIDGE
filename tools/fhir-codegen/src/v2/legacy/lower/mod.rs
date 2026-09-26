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

mod listing;
mod lowering;
mod segment;

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::legacy::datatype::{LegacyDataType, lower_data_types};
use crate::v2::legacy::lower::listing::base_code;
use crate::v2::legacy::lower::listing::listing;
use crate::v2::legacy::lower::listing::placed_segments;
use crate::v2::legacy::lower::listing::withdrawals;
use crate::v2::legacy::lower::lowering::Lowering;
use crate::v2::legacy::source::{Table0354, Tables, version_key};
use crate::v2::lower::{Model, Segment, Structure};

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
