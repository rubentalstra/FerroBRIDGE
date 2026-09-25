// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering the legacy tables to the shapes of [`crate::v2::lower`].
//!
//! The root set is every message structure a version's `messages.json` lists
//! whose code no structure of the v2.9.1 definitions carries (variants aside:
//! `ORU_R01-A` carries `ORU_R01`), taken from every version whose
//! `groups.json` gives it a root. A structure's tree is the elements under its
//! root group, ordered by `(position, id)` among siblings and renumbered from
//! 1, with element ids derived by the rule of the v2.9.1 definitions
//! (`ORM_O01.2-PATIENT.1-PID`, `choice-n-NAME` under a choice). Each segment
//! a tree names is lowered from the same version's `segments.json`,
//! `fields.json` and `data_elements.json`; one whose field table agrees with
//! the v2.9.1 segment is linked to that segment instead. A structure is marked
//! withdrawn as of the first version, among the extract's versions and then
//! v2.9.1, that lists it no more, and table 0354 must mark its code
//! deprecated.
//!
//! Each defect of the export is tolerated only where it was found
//! ([`LegacyDefect::tolerated_in`]); the same defect anywhere else is refused.

use std::collections::{BTreeMap, BTreeSet};

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
            // NOTE: no specification governs this: our own design; elements.json
            // numbers the elements of a version in one sequence, and these files repeat
            // a number among siblings (ADR_A19.ROOT at 2.4), so siblings order by (position, id).
            Self::RepeatedPosition => &[
                "2.4/elements.json",
                "2.5/elements.json",
                "2.5.1/elements.json",
                "2.6/elements.json",
                "2.7.1/elements.json",
            ],
            // NOTE: these files list structures with no root group (2.3 has trees for
            // 10 of its 114 structures); the structure is taken from the versions that have one.
            Self::MessageWithoutTree => &[
                "2.3/messages.json",
                "2.3.1/messages.json",
                "2.4/messages.json",
            ],
            // NOTE: these files carry group rows no element places (a second
            // ORM_O01.ORDER at 2.3); the tree is the one its root reaches.
            Self::UnreachedGroup => &["2.3/groups.json", "2.5/groups.json", "2.5.1/groups.json"],
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

/// One version's structures and the segments they name.
#[derive(Debug, Clone)]
pub struct LegacyVersion {
    /// The version, for example `2.5.1`.
    pub version: String,
    /// The structures of the root set this version carries, by id.
    pub structures: BTreeMap<String, Structure>,
    /// The segments the trees name whose field table differs from the v2.9.1
    /// segment of that id (or that v2.9.1 lacks), by id.
    pub segments: BTreeMap<String, Segment>,
    /// The segments the trees name whose field table agrees with the v2.9.1
    /// segment, which the trees link to instead.
    pub shared: BTreeSet<String>,
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
        let mut versions = Vec::new();
        let mut messages = Vec::new();
        for (index, version) in tables.versions().iter().enumerate() {
            let Some(roots) = rooted.get(index) else {
                continue;
            };
            let mut lowered = Lowering::new(version, &mut hits);
            let mut structures = BTreeMap::new();
            for message in &version.messages {
                let Some(withdrawn_as_of) = withdrawn.get(&message.id) else {
                    continue;
                };
                let Some(root) = roots.get(message.id.as_str()) else {
                    continue;
                };
                let structure = lowered.structure(&message.id, root, withdrawn_as_of)?;
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
            let (segments, shared) = lowered.segments(current)?;
            versions.push(LegacyVersion {
                version: version.version.clone(),
                structures,
                segments,
                shared,
            });
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

    /// The number of legacy structures over every version.
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

    /// The number of legacy segments emitted over every version.
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.versions.iter().map(|v| v.segments.len()).sum()
    }

    /// The number of legacy segment references linked to a v2.9.1 segment.
    #[must_use]
    pub fn shared_count(&self) -> usize {
        self.versions.iter().map(|v| v.shared.len()).sum()
    }

    /// The number of fields over every emitted legacy segment.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.versions
            .iter()
            .flat_map(|v| v.segments.values())
            .map(|segment| segment.fields.len())
            .sum()
    }
}

/// The versions that list each structure code `current` lacks, by the index
/// of the version, and the root group of each such structure per version.
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
        let roots = roots(version, current)?;
        for message in &version.messages {
            if current.contains(message.id.as_str()) {
                continue;
            }
            let expected = format!("{}_{}", message.msg_type_id, message.event_id);
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
            listed.entry(message.id.clone()).or_default().push(index);
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

/// The root group of each structure of `version` whose code is not in
/// `current`, refusing a second root of such a structure.
fn roots<'a>(
    version: &'a VersionTables,
    current: &BTreeSet<&str>,
) -> Result<BTreeMap<String, &'a GroupRow>, LegacyError> {
    let mut out = BTreeMap::new();
    for group in version.groups.iter().filter(|group| group.is_root) {
        if current.contains(group.message_id.as_str()) {
            continue;
        }
        if out.insert(group.message_id.clone(), group).is_some() {
            return Err(invalid(
                &version.file("groups"),
                &group.name,
                "a second root group of its structure",
            ));
        }
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

/// The lowering of one version: its trees, and the segments they reach.
struct Lowering<'a, 'h> {
    version: &'a VersionTables,
    hits: &'h mut BTreeSet<(LegacyDefect, String)>,
    groups: BTreeMap<u64, &'a GroupRow>,
    children: BTreeMap<u64, Vec<&'a ElementRow>>,
    reached: BTreeSet<String>,
}

impl<'a, 'h> Lowering<'a, 'h> {
    fn new(version: &'a VersionTables, hits: &'h mut BTreeSet<(LegacyDefect, String)>) -> Self {
        let groups = version
            .groups
            .iter()
            .map(|group| (group.id, group))
            .collect();
        let mut children: BTreeMap<u64, Vec<&ElementRow>> = BTreeMap::new();
        for element in &version.elements {
            children.entry(element.parent_id).or_default().push(element);
        }
        for siblings in children.values_mut() {
            siblings.sort_by_key(|element| (element.position, element.id));
        }
        Self {
            version,
            hits,
            groups,
            children,
            reached: BTreeSet::new(),
        }
    }

    fn structure(
        &mut self,
        id: &str,
        root: &GroupRow,
        withdrawn_as_of: &str,
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
            withdrawn_as_of: Some(withdrawn_as_of.to_owned()),
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
        let mut previous = None;
        for (index, element) in elements.iter().enumerate() {
            let row = element.id.to_string();
            self.check_element(group, element, previous)?;
            previous = Some(element.position);
            let position =
                u16::try_from(index.saturating_add(1)).map_err(|source| LegacyError::Range {
                    file: elements_file.clone(),
                    row: row.clone(),
                    source,
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

    /// Checks one element of `group`: its position against the sibling
    /// before it, its usage against its minimum, and a choice member's usage.
    fn check_element(
        &mut self,
        group: &GroupRow,
        element: &ElementRow,
        previous: Option<u64>,
    ) -> Result<(), LegacyError> {
        let file = self.version.file("elements");
        let row = element.id.to_string();
        if previous == Some(element.position) {
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

    /// The segments the trees reached: those that differ from the v2.9.1
    /// segment of their id, and the ids of those that agree.
    fn segments(
        self,
        current: &Model,
    ) -> Result<(BTreeMap<String, Segment>, BTreeSet<String>), LegacyError> {
        let version = self.version;
        let elements: BTreeMap<&str, &DataElementRow> = version
            .data_elements
            .iter()
            .map(|element| (element.id.as_str(), element))
            .collect();
        let mut differing = BTreeMap::new();
        let mut shared = BTreeSet::new();
        for id in &self.reached {
            let segment = lower_segment(version, id, &elements, self.hits)?;
            match current.segments.get(id) {
                Some(defined) if same_table(&segment, defined) => {
                    shared.insert(id.clone());
                }
                _ => {
                    differing.insert(id.clone(), segment);
                }
            }
        }
        Ok((differing, shared))
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
    let mut fields = Vec::with_capacity(rows.len());
    for (index, (position, field)) in rows.into_iter().enumerate() {
        let label = format!("{id}.{position}");
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
        fields.push(lower_field(version, label, position, field, element)?);
    }
    Ok(Segment {
        id: id.to_owned(),
        url: None,
        name: row.description.clone(),
        fields,
    })
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
        "" if element.min_length == 0 => None,
        "" => Some(Length {
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
