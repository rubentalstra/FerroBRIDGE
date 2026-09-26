// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The group matching: which message map rows name a placed segment whose
//! group path the rows spell otherwise.

use std::collections::{BTreeMap, BTreeSet};

use crate::map::Outcome;
use crate::map::corpus::{Map, Row};
use crate::map::run::{Run, Visit};
use crate::parse::Location;

impl<'a> Run<'a> {
    /// The message map rows naming a placed segment: the rows whose source is
    /// its path, else the rows of the one source [`regrouped`] reaches it
    /// by, counted as `group-path` when a group is added or omitted and as
    /// `group-renamed` per group paired under another name; `None` when no
    /// row names it.
    pub(super) fn rows(
        &mut self,
        map: &'a Map,
        visit: &Visit<'a>,
        at: &Location,
    ) -> Option<Vec<&'a Row>> {
        let rows: Vec<&Row> = map
            .rows
            .iter()
            .filter(|row| visit.matches(&row.source))
            .collect();
        if !rows.is_empty() {
            return Some(rows);
        }
        let structure = self.parsed.structure_name();
        let nodes = self.parsed.structure().nodes;
        let tree = tree_paths(structure, nodes);
        let contents = Contents {
            rows: row_contents(map),
            tree: tree_contents(structure, nodes),
        };
        let Some(found) = regrouped(map, &visit.code, &tree, &contents) else {
            let source = unwrapped(map, &visit.code, &tree, &contents)?;
            self.outcomes.push(Outcome::GroupPath {
                at: at.clone(),
                row_path: String::from(source),
                tree_path: visit.code.clone(),
            });
            return Some(map.rows.iter().filter(|row| row.source == source).collect());
        };
        if found.shifted {
            self.outcomes.push(Outcome::GroupPath {
                at: at.clone(),
                row_path: String::from(found.source),
                tree_path: visit.code.clone(),
            });
        }
        for (row_group, tree_group) in found.renamed {
            self.outcomes.push(Outcome::GroupRenamed {
                at: at.clone(),
                row_path: String::from(found.source),
                tree_path: visit.code.clone(),
                row_group,
                tree_group,
            });
        }
        Some(
            map.rows
                .iter()
                .filter(|row| row.source == found.source)
                .collect(),
        )
    }
}

/// The path of every segment node of a structure's tree, as a message map
/// row names it: `ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR`.
fn tree_paths(structure: &str, nodes: &'static [hl7v2_types::model::Node]) -> BTreeSet<String> {
    fn walk(prefix: &str, nodes: &'static [hl7v2_types::model::Node], out: &mut BTreeSet<String>) {
        for node in nodes {
            match node {
                hl7v2_types::model::Node::Segment(reference) => {
                    out.insert(format!("{prefix}.{}", reference.segment.id));
                }
                hl7v2_types::model::Node::Group(group) => {
                    walk(&format!("{prefix}.{}", group.name), group.children, out);
                }
                hl7v2_types::model::Node::Placeholder(_) => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(structure, nodes, &mut out);
    out
}

/// The segment ids under each group of a structure's tree, directly or in
/// its child groups, keyed by the group's path (`ORM_O01.PATIENT`).
fn tree_contents(
    structure: &str,
    nodes: &'static [hl7v2_types::model::Node],
) -> BTreeMap<String, BTreeSet<&'static str>> {
    fn walk(
        prefix: &str,
        nodes: &'static [hl7v2_types::model::Node],
        out: &mut BTreeMap<String, BTreeSet<&'static str>>,
    ) -> BTreeSet<&'static str> {
        let mut held = BTreeSet::new();
        for node in nodes {
            match node {
                hl7v2_types::model::Node::Segment(reference) => {
                    held.insert(reference.segment.id);
                }
                hl7v2_types::model::Node::Group(group) => {
                    let path = format!("{prefix}.{}", group.name);
                    let inner = walk(&path, group.children, out);
                    held.extend(inner.iter().copied());
                    out.entry(path).or_default().extend(inner);
                }
                hl7v2_types::model::Node::Placeholder(_) => {}
            }
        }
        held
    }
    let mut out = BTreeMap::new();
    walk(structure, nodes, &mut out);
    out
}

/// The segment ids a message map's rows place under each group path of
/// their sources, keyed as [`tree_contents`] keys a tree's.
fn row_contents(map: &Map) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in &map.rows {
        if row.source.contains(":follow:") {
            continue;
        }
        let Some((groups, segment)) = row.source.rsplit_once('.') else {
            continue;
        };
        let mut prefix = groups;
        while let Some((parent, _)) = prefix.rsplit_once('.') {
            out.entry(String::from(prefix))
                .or_default()
                .insert(String::from(segment));
            prefix = parent;
        }
    }
    out
}

/// The segments under each group, of the guide's rows and of the tree.
#[derive(Debug, Default)]
struct Contents {
    rows: BTreeMap<String, BTreeSet<String>>,
    tree: BTreeMap<String, BTreeSet<&'static str>>,
}

impl Contents {
    /// Whether the tree group at `tree` holds every segment the rows place in
    /// the row group at `row`, one of them at least.
    fn holds(&self, row: &str, tree: &str) -> bool {
        match (self.rows.get(row), self.tree.get(tree)) {
            (Some(placed), Some(held)) => {
                !placed.is_empty() && placed.iter().all(|id| held.contains(id.as_str()))
            }
            _ => false,
        }
    }
}

/// How a row source's groups pair with a tree path's.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pairing {
    /// Whether one group is added or omitted.
    shifted: bool,
    /// Each row group paired with a tree group of another name, as `(row,
    /// tree)` in path order.
    renamed: Vec<(String, String)>,
}

/// A row source reaching a parsed segment's path, with how its groups pair.
#[derive(Debug, PartialEq, Eq)]
struct Regrouping<'m> {
    source: &'m str,
    shifted: bool,
    renamed: Vec<(String, String)>,
}

/// The one message map row source that reaches the parsed segment's `path`
/// among the `tree` paths of its structure.
///
/// The guide's message maps name the group paths of one structure per
/// message (`ORM_O01.ORDER_DETAIL.CHOICE.OBR`, `ORM_O01.PATIENT.VISIT.PV1`),
/// while a legacy tree nests and names the groups of its own version
/// (`ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR` and
/// `ORM_O01.PATIENT.PATIENT_VISIT.PV1` at 2.3). A row source reaches a path
/// when [`pairing`] pairs their groups, and the pairing with the fewest
/// renamed groups counts. A row source any tree path names exactly belongs
/// to that path. The match holds only when one row source reaches `path` at
/// the fewest renames and that source reaches no other tree path at as few;
/// otherwise the segment stays unmapped. The rule holds for every tree,
/// legacy and v2.9.1 alike, since the guide's paths also disagree with the
/// v2.9.1 definitions (`OML_O21`). No specification governs this: our own
/// design, since `mapping_guidelines.md` writes one structure per message.
fn regrouped<'m>(
    map: &'m Map,
    path: &str,
    tree: &BTreeSet<String>,
    contents: &Contents,
) -> Option<Regrouping<'m>> {
    let sources: BTreeSet<&str> = map
        .rows
        .iter()
        .map(|row| row.source.as_str())
        .filter(|source| !source.contains(":follow:") && !tree.contains(*source))
        .collect();
    let (source, found) = cheapest(
        sources
            .into_iter()
            .filter_map(|source| pairing(source, path, contents).map(|found| (source, found))),
    )?;
    let (only, _) = cheapest(
        tree.iter()
            .filter_map(|other| pairing(source, other, contents).map(|found| (other, found))),
    )?;
    (only == path).then_some(Regrouping {
        source,
        shifted: found.shifted,
        renamed: found.renamed,
    })
}

/// The one message map row source whose innermost group the parsed tree
/// omits, placing the segment directly in that group's parent.
///
/// The row source must name the tree `path`'s structure, groups and segment
/// with one group more at the end, the tree must carry no group of that name
/// in the parent, no row may name `path` itself, and the source must reach no
/// other tree path ([`pairing`]). The guide's
/// `ORU_R01.PATIENT_RESULT.ORDER_OBSERVATION.COMMON_ORDER.ORC` so reaches the
/// 2.5.1 ORC, which its tables place in `ORDER_OBSERVATION` with no
/// `COMMON_ORDER` group. No specification governs this: our own design.
fn unwrapped<'m>(
    map: &'m Map,
    path: &str,
    tree: &BTreeSet<String>,
    contents: &Contents,
) -> Option<&'m str> {
    if map.rows.iter().any(|row| row.source == path) {
        return None;
    }
    let (parent, segment) = path.rsplit_once('.')?;
    let sources: BTreeSet<&str> = map
        .rows
        .iter()
        .map(|row| row.source.as_str())
        .filter(|source| !source.contains(":follow:") && !tree.contains(*source))
        .filter(|source| {
            source
                .strip_prefix(parent)
                .and_then(|rest| rest.strip_prefix('.'))
                .and_then(|rest| rest.strip_suffix(segment))
                .and_then(|rest| rest.strip_suffix('.'))
                .is_some_and(|group| {
                    !group.is_empty()
                        && !group.contains('.')
                        && !contents.tree.contains_key(&format!("{parent}.{group}"))
                })
        })
        .collect();
    let mut sources = sources.into_iter();
    let Some(source) = sources.next() else {
        return wrapped(map, path, tree, contents);
    };
    if sources.next().is_some() {
        return None;
    }
    let elsewhere = tree
        .iter()
        .any(|other| other != path && pairing(source, other, contents).is_some());
    (!elsewhere).then_some(source)
}

/// The one message map row source that places the segment directly in the
/// parent of the tree `path`'s innermost group, which the rows never name.
///
/// It mirrors [`unwrapped`], for a tree that adds the innermost group where
/// that one serves a tree that omits it: the supplement's
/// `ORL_O22.RESPONSE.PID` reaches the 2.5.1 `ORL_O22.RESPONSE.PATIENT.PID`.
/// The source must reach no other tree path, and no other tree path may
/// wrap the segment in another group of the same parent. No specification
/// governs this: our own design.
fn wrapped<'m>(
    map: &'m Map,
    path: &str,
    tree: &BTreeSet<String>,
    contents: &Contents,
) -> Option<&'m str> {
    let (parent, segment) = path.rsplit_once('.')?;
    let (grand, group) = parent.rsplit_once('.')?;
    if contents.rows.contains_key(parent) {
        return None;
    }
    let wanted = format!("{grand}.{segment}");
    let source = map
        .rows
        .iter()
        .map(|row| row.source.as_str())
        .find(|source| *source == wanted && !tree.contains(*source))?;
    let rivals = tree.iter().any(|other| {
        other != path
            && (pairing(source, other, contents).is_some()
                || other
                    .strip_prefix(grand)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .and_then(|rest| rest.strip_suffix(segment))
                    .and_then(|rest| rest.strip_suffix('.'))
                    .is_some_and(|other_group| other_group != group && !other_group.contains('.')))
    });
    (!rivals).then_some(source)
}

/// The one candidate with the fewest renamed groups; `None` when there is
/// none or two share the fewest.
fn cheapest<T>(candidates: impl Iterator<Item = (T, Pairing)>) -> Option<(T, Pairing)> {
    let candidates: Vec<(T, Pairing)> = candidates.collect();
    let least = candidates
        .iter()
        .map(|(_, found)| found.renamed.len())
        .min()?;
    let mut best = candidates
        .into_iter()
        .filter(|(_, found)| found.renamed.len() == least);
    let first = best.next()?;
    best.next().is_none().then_some(first)
}

/// The path of a group: the structure and the groups up to `index`.
fn group_path(structure: &str, groups: &[&str], index: usize) -> String {
    let mut out = String::from(structure);
    for group in groups.iter().take(index.saturating_add(1)) {
        out.push('.');
        out.push_str(group);
    }
    out
}

/// The ways to pair `rows` groups with `trees` groups in order, as index
/// pairs, when one side holds at most one group more and that group is not
/// the last.
fn alignments(rows: usize, trees: usize) -> Vec<Vec<(usize, usize)>> {
    let skip = |index: usize, gap: usize| {
        if index < gap {
            index
        } else {
            index.saturating_add(1)
        }
    };
    if rows == trees {
        vec![(0..rows).map(|index| (index, index)).collect()]
    } else if trees == rows.saturating_add(1) {
        (0..rows)
            .map(|gap| (0..rows).map(|index| (index, skip(index, gap))).collect())
            .collect()
    } else if rows == trees.saturating_add(1) {
        (0..trees)
            .map(|gap| (0..trees).map(|index| (skip(index, gap), index)).collect())
            .collect()
    } else {
        Vec::new()
    }
}

/// How the groups of a row `source` pair with those of a tree `path`, when
/// they name one structure and one segment.
///
/// The groups pair in order, one of the two paths holding at most one group
/// more, which is not the group holding the segment. A pair names one group,
/// or two names when the tree group holds every segment the guide's rows
/// place in the row group ([`Contents::holds`]), which pairs the guide's
/// `ORM_O01.PATIENT.VISIT` with the 2.3 `ORM_O01.PATIENT.PATIENT_VISIT`. Of
/// the ways to pair, the one with the fewest renamed groups counts, and two
/// that differ leave the paths unpaired.
fn pairing(source: &str, path: &str, contents: &Contents) -> Option<Pairing> {
    let row: Vec<&str> = source.split('.').collect();
    let tree: Vec<&str> = path.split('.').collect();
    let ((row_structure, row_rest), (tree_structure, tree_rest)) =
        (row.split_first()?, tree.split_first()?);
    let ((row_segment, row_groups), (tree_segment, tree_groups)) =
        (row_rest.split_last()?, tree_rest.split_last()?);
    if row_structure != tree_structure || row_segment != tree_segment {
        return None;
    }
    let shifted = row_groups.len() != tree_groups.len();
    let mut found: Vec<Pairing> = Vec::new();
    for alignment in alignments(row_groups.len(), tree_groups.len()) {
        let mut renamed = Vec::new();
        let paired = alignment.iter().all(|&(at_row, at_tree)| {
            let (Some(named), Some(held)) = (row_groups.get(at_row), tree_groups.get(at_tree))
            else {
                return false;
            };
            if named == held {
                return true;
            }
            let holds = contents.holds(
                &group_path(row_structure, row_groups, at_row),
                &group_path(tree_structure, tree_groups, at_tree),
            );
            if holds {
                renamed.push((String::from(*named), String::from(*held)));
            }
            holds
        });
        let candidate = Pairing { shifted, renamed };
        if paired && !found.contains(&candidate) {
            found.push(candidate);
        }
    }
    cheapest(found.into_iter().map(|found| ((), found))).map(|((), found)| found)
}

#[cfg(test)]
mod tests {
    use super::{Contents, pairing, regrouped, row_contents, tree_contents, tree_paths};
    use crate::map::Outcome;
    use crate::map::run::Run;
    use crate::map::run::tests::{corpus, legacy_order, message, writes_of};
    use crate::parse::Parsed;

    #[test]
    fn a_legacy_order_detail_reaches_the_guides_rows_through_its_group_path() {
        let parsed = legacy_order();
        assert_eq!(parsed.structure().version, "2.3");
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let matched: Vec<(&str, &str)> = run
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::GroupPath {
                    row_path,
                    tree_path,
                    ..
                } => Some((row_path.as_str(), tree_path.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            matched,
            [
                (
                    "ORM_O01.ORDER_DETAIL.CHOICE.OBR",
                    "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR"
                ),
                ("ORM_O01.ORDER_DETAIL.NTE", "ORM_O01.ORDER.ORDER_DETAIL.NTE"),
                ("ORM_O01.ORDER_DETAIL.DG1", "ORM_O01.ORDER.ORDER_DETAIL.DG1"),
                (
                    "ORM_O01.ORDER_DETAIL.OBSERVATION.OBX",
                    "ORM_O01.ORDER.ORDER_DETAIL.OBSERVATION.OBX"
                ),
                (
                    "ORM_O01.ORDER_DETAIL.CHOICE.RXO",
                    "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.RXO"
                ),
            ]
        );
        let unmapped: Vec<&Outcome> = run
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::UnmappedSegment { .. }))
            .collect();
        assert!(unmapped.is_empty(), "{unmapped:?}");
        for type_name in ["ServiceRequest", "Observation", "MedicationRequest"] {
            assert!(
                !writes_of(&run, type_name).is_empty(),
                "the order detail writes a {type_name}"
            );
        }
    }

    #[test]
    fn a_group_path_that_reaches_two_tree_paths_stays_unmapped() {
        let corpus = corpus();
        let map = corpus
            .message_map("ORM_O01")
            .expect("the ORM_O01 message map");
        let obr = "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR";
        let none = Contents::default();
        let one: std::collections::BTreeSet<String> = [String::from(obr)].into();
        assert_eq!(
            regrouped(map, obr, &one, &none).map(|found| found.source),
            Some("ORM_O01.ORDER_DETAIL.CHOICE.OBR")
        );
        let two: std::collections::BTreeSet<String> = [
            String::from(obr),
            String::from("ORM_O01.ORDER_DETAIL.EXTRA.CHOICE.OBR"),
        ]
        .into();
        assert_eq!(regrouped(map, obr, &two, &none), None);
        let top: std::collections::BTreeSet<String> = [String::from("ORM_O01.NTE")].into();
        assert_eq!(regrouped(map, "ORM_O01.NTE", &top, &none), None);
    }

    /// Whether two paths pair by one group added or omitted, with no group
    /// renamed.
    fn one_group_apart(left: &str, right: &str) -> bool {
        pairing(left, right, &Contents::default())
            .is_some_and(|found| found.shifted && found.renamed.is_empty())
    }

    /// A synthetic 2.3 ORM^O01 whose patient carries a visit.
    fn legacy_visit() -> Parsed {
        message(&[
            "MSH|^~\\&|NORTHLAB|NORTHHOSP|EHR|SOUTHCLINIC|20260925143000+0200||ORM^O01|MSG00032|P|2.3",
            "PID|1||PAT-0032^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            "PV1|1|I|WARD-3^ROOM-12^BED-1",
            "PV2|||^Observation",
            "ORC|NW|PLC-33",
            "OBR|1|PLC-33||2345-7^Glucose^LN",
        ])
    }

    /// The `group-renamed` outcomes of a run, as row path, tree path, row
    /// group and tree group.
    fn renamed(run: &Run<'_>) -> Vec<(String, String, String, String)> {
        run.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::GroupRenamed {
                    row_path,
                    tree_path,
                    row_group,
                    tree_group,
                    ..
                } => Some((
                    row_path.clone(),
                    tree_path.clone(),
                    row_group.clone(),
                    tree_group.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_legacy_patient_visit_reaches_the_guides_visit_rows_by_its_segments() {
        let parsed = legacy_visit();
        assert_eq!(parsed.structure().version, "2.3");
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        let pair = |segment: &str| {
            (
                format!("ORM_O01.PATIENT.VISIT.{segment}"),
                format!("ORM_O01.PATIENT.PATIENT_VISIT.{segment}"),
                String::from("VISIT"),
                String::from("PATIENT_VISIT"),
            )
        };
        assert_eq!(renamed(&run), [pair("PV1"), pair("PV2")]);
        let unmapped: Vec<&Outcome> = run
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Outcome::UnmappedSegment { .. }))
            .collect();
        assert!(unmapped.is_empty(), "{unmapped:?}");
        assert!(
            !writes_of(&run, "Encounter").is_empty(),
            "the visit writes an Encounter"
        );
    }

    #[test]
    fn a_renamed_and_a_shifted_group_in_one_path_pair_uniquely() {
        let corpus = corpus();
        let map = corpus
            .message_map("ORM_O01")
            .expect("the ORM_O01 message map");
        let pv1 = "ORM_O01.PATIENT.EXTRA.PATIENT_VISIT.PV1";
        let tree: std::collections::BTreeSet<String> = [
            String::from(pv1),
            String::from("ORM_O01.PATIENT.EXTRA.PATIENT_VISIT.PV2"),
        ]
        .into();
        let held: std::collections::BTreeSet<&'static str> = ["PV1", "PV2"].into();
        let contents = Contents {
            rows: row_contents(map),
            tree: [
                (String::from("ORM_O01.PATIENT"), held.clone()),
                (String::from("ORM_O01.PATIENT.EXTRA"), held.clone()),
                (String::from("ORM_O01.PATIENT.EXTRA.PATIENT_VISIT"), held),
            ]
            .into(),
        };
        let found = regrouped(map, pv1, &tree, &contents).expect("the visit rows reach PV1");
        assert_eq!(found.source, "ORM_O01.PATIENT.VISIT.PV1");
        assert!(found.shifted);
        assert_eq!(
            found.renamed,
            [(String::from("VISIT"), String::from("PATIENT_VISIT"))]
        );
    }

    #[test]
    fn two_tree_groups_holding_the_guides_segments_leave_the_segment_unmapped() {
        let corpus = corpus();
        let map = corpus
            .message_map("ORM_O01")
            .expect("the ORM_O01 message map");
        let first = "ORM_O01.PATIENT.PATIENT_VISIT.PV1";
        let tree: std::collections::BTreeSet<String> = [
            String::from(first),
            String::from("ORM_O01.PATIENT.PATIENT_VISIT.PV2"),
            String::from("ORM_O01.PATIENT.OTHER_VISIT.PV1"),
            String::from("ORM_O01.PATIENT.OTHER_VISIT.PV2"),
        ]
        .into();
        let held: std::collections::BTreeSet<&'static str> = ["PV1", "PV2"].into();
        let contents = Contents {
            rows: row_contents(map),
            tree: [
                (String::from("ORM_O01.PATIENT.PATIENT_VISIT"), held.clone()),
                (String::from("ORM_O01.PATIENT.OTHER_VISIT"), held),
            ]
            .into(),
        };
        assert_eq!(regrouped(map, first, &tree, &contents), None);
        let lacking = Contents {
            rows: row_contents(map),
            tree: [(
                String::from("ORM_O01.PATIENT.PATIENT_VISIT"),
                ["PV1"].into(),
            )]
            .into(),
        };
        let only: std::collections::BTreeSet<String> = [String::from(first)].into();
        assert_eq!(
            regrouped(map, first, &only, &lacking),
            None,
            "a tree group lacking the guide's PV2 is no rename"
        );
    }

    #[test]
    fn a_current_result_renames_no_group() {
        let parsed = message(&[
            "MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00033|P|2.9.1",
            "PID|1||PAT-0033^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
            "PV1|1|O",
            "ORC|RE|PLC-1|FIL-1",
            "OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
            "NTE|1||Fasting sample",
            "OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L",
            "SPM|1|SPC-1||BLD^Blood^HL70487",
        ]);
        assert_eq!(parsed.structure().version, "2.9.1");
        let corpus = corpus();
        let mut run = Run::new(&corpus, &parsed);
        run.message().expect("the walk runs");
        assert!(renamed(&run).is_empty(), "{:?}", renamed(&run));
    }

    #[test]
    fn the_tree_contents_hold_the_segments_of_each_group() {
        let structure = hl7v2_types::legacy::find("ORM_O01", "2.3").expect("ORM_O01 at 2.3");
        let contents = tree_contents("ORM_O01", structure.nodes);
        let visit = contents
            .get("ORM_O01.PATIENT.PATIENT_VISIT")
            .expect("the 2.3 visit group");
        assert_eq!(visit.iter().copied().collect::<Vec<_>>(), ["PV1", "PV2"]);
        let patient = contents.get("ORM_O01.PATIENT").expect("the patient group");
        assert!(patient.contains("PID") && patient.contains("PV1"));
    }

    #[test]
    fn two_paths_one_group_apart_share_the_group_holding_the_segment() {
        let row = "ORM_O01.ORDER_DETAIL.CHOICE.OBR";
        assert!(one_group_apart(
            row,
            "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR"
        ));
        assert!(one_group_apart(row, "ORM_O01.CHOICE.OBR"));
        assert!(!one_group_apart(row, "ORM_O01.ORDER_DETAIL.OBR"));
        assert!(!one_group_apart(row, "ORM_O01.ORDER_DETAIL.CHOICE.X.OBR"));
        assert!(!one_group_apart(row, "ORM_O01.A.B.ORDER_DETAIL.CHOICE.OBR"));
        assert!(!one_group_apart(
            row,
            "ORM_O01.ORDER.ORDER_DETAIL.CHOICE.RXO"
        ));
        assert!(!one_group_apart(
            row,
            "ORU_R01.ORDER.ORDER_DETAIL.CHOICE.OBR"
        ));
        assert!(!one_group_apart("ORM_O01.ORDER_DETAIL.NTE", "ORM_O01.NTE"));
    }

    #[test]
    fn the_tree_paths_name_every_segment_node_by_its_groups() {
        let structure = hl7v2_types::legacy::find("ORM_O01", "2.3").expect("ORM_O01 at 2.3");
        let paths = tree_paths("ORM_O01", structure.nodes);
        assert!(paths.contains("ORM_O01.MSH"));
        assert!(paths.contains("ORM_O01.ORDER.ORDER_DETAIL.CHOICE.OBR"));
        assert!(paths.contains("ORM_O01.ORDER.ORDER_DETAIL.OBSERVATION.OBX"));
    }
}
