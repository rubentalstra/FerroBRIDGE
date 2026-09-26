// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering one version: its segments linked or emitted, and its trees.

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::legacy::datatype::lower_data_types;
use crate::v2::legacy::lower::LegacyDefect;
use crate::v2::legacy::lower::LegacyError;
use crate::v2::legacy::lower::LegacyVersion;
use crate::v2::legacy::lower::Owner;
use crate::v2::legacy::lower::invalid;
use crate::v2::legacy::lower::segment::cardinality;
use crate::v2::legacy::lower::segment::lower_segment;
use crate::v2::legacy::lower::segment::same_table;
use crate::v2::legacy::lower::tolerate;
use crate::v2::legacy::source::{DataElementRow, ElementRow, GroupRow, VersionTables};
use crate::v2::lower::{GroupKind, Model, Node, Segment, Structure};

/// The segments a version emits, and the static each other segment it names
/// links to.
type SegmentLinks = (BTreeMap<String, Segment>, BTreeMap<String, Owner>);

/// The lowering of one version: its trees, and the segments they reach.
pub(super) struct Lowering<'a, 'h> {
    version: &'a VersionTables,
    pub(super) hits: &'h mut BTreeSet<(LegacyDefect, String)>,
    groups: BTreeMap<u64, &'a GroupRow>,
    children: BTreeMap<u64, Vec<&'a ElementRow>>,
    /// The structures one of whose elements places a group of another.
    hosts: BTreeSet<&'a str>,
    reached: BTreeSet<String>,
}

impl<'a, 'h> Lowering<'a, 'h> {
    pub(super) fn new(
        version: &'a VersionTables,
        hits: &'h mut BTreeSet<(LegacyDefect, String)>,
    ) -> Self {
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
    pub(super) fn damage(&self, id: &str, root: &GroupRow) -> Option<(LegacyDefect, &'static str)> {
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

    pub(super) fn structure(
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
    pub(super) fn segments(
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
