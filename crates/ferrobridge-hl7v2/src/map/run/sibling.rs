// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The sibling families of a created resource and the settling of the
//! references between them.

use std::collections::{BTreeMap, BTreeSet};

use fhir_types::codec::Value;

use crate::map::corpus::Map;
use crate::map::notation::{Label, Step, Target};
use crate::map::run::{Base, Identity, Pending, Run, Write};
use crate::map::{Outcome, RowRef};
use crate::parse::Location;

impl Run<'_> {
    /// The anchor of the sibling family `base` lies in: the created resource
    /// whose root `base` is, or whose sibling it is, when the row's map names
    /// instances of that resource's own type ([`names_siblings`]).
    ///
    /// `datatype-pl-to-location` fills one `Location` per PL level through
    /// `[1].` to `[6].` and links them by `partOf.reference(Location[k])`;
    /// the prefix applies where the data type is used (`mapping_guidelines.md`
    /// §\[n\] Notation), which for a map rooted at a resource is the
    /// resource. No specification governs the family: our own design.
    pub(super) fn family(&self, base: &Base, reference: &RowRef) -> Option<usize> {
        if !base.slots.is_empty() {
            return None;
        }
        let resource = self.resources.get(base.resource)?;
        let anchor = match resource.identity {
            Identity::Created { .. } => base.resource,
            Identity::Sibling { anchor, .. } => anchor,
            Identity::Envelope | Identity::Message { .. } => return None,
        };
        let map = self.corpus.get(&reference.map)?;
        names_siblings(map, &resource.type_name).then_some(anchor)
    }

    /// The `label` sibling of `anchor`, created on first sight.
    pub(super) fn sibling(&mut self, anchor: usize, label: u32) -> usize {
        let identity = Identity::Sibling { anchor, label };
        if let Some(index) = self
            .resources
            .iter()
            .position(|resource| resource.identity == identity)
        {
            return index;
        }
        let type_name = self
            .resources
            .get(anchor)
            .map(|resource| resource.type_name.clone())
            .unwrap_or_default();
        self.create(&type_name, identity)
    }

    /// Records a `(Type[k])` reference from a member of a sibling family to
    /// its `k` sibling, answering whether the row's target is one.
    pub(super) fn sibling_reference(
        &mut self,
        base: &Base,
        steps: &[Step],
        repetition: usize,
        at: &Location,
        reference: &RowRef,
    ) -> bool {
        let Some(target) = steps.last().and_then(|step| step.reference.as_ref()) else {
            return false;
        };
        let Some(Label::Number(label)) = target.label else {
            return false;
        };
        if !target.path.is_empty() {
            return false;
        }
        let Some(anchor) = self.family(base, reference) else {
            return false;
        };
        let own_type = self
            .resources
            .get(base.resource)
            .is_some_and(|resource| resource.type_name == target.resource);
        if !own_type {
            return false;
        }
        let Some(slots) = self.slots(base, steps, repetition, at, reference) else {
            return true;
        };
        let mut leaf = Base {
            resource: base.resource,
            slots,
        };
        let Some(resolved) = self.resolve(&leaf, at, reference) else {
            return true;
        };
        if matches!(
            resolved.location(),
            fhirconnect::tree::element::Location::Complex(schema) if schema.name == "Reference"
        ) {
            leaf = leaf.child("reference", "");
        }
        let pending = Pending::Sibling {
            anchor,
            label,
            map: reference.map.clone(),
        };
        self.record(&leaf, pending, at, reference);
        true
    }

    /// Settles every sibling family: each reference between siblings, and
    /// the level every reference to the family takes.
    ///
    /// A reference to a sibling no value reached takes the sibling that
    /// one's own row names, up to one a value reached, so a PL without a
    /// floor has its point of care in its building. The references to the
    /// family take its finest level, the one valued sibling no other valued
    /// sibling is part of; when that is not one sibling, they stay at the
    /// anchor, the resource the guide's `(Type)` names, and a counted
    /// outcome says so. No specification governs this: our own design, over
    /// FHIR R4 `Location.partOf` (<https://hl7.org/fhir/R4/location.html>).
    pub(super) fn siblings(&mut self) {
        let mut anchors = BTreeSet::new();
        for resource in &self.resources {
            if let Identity::Sibling { anchor, .. } = resource.identity {
                anchors.insert(anchor);
            }
            for write in &resource.writes {
                if let Pending::Sibling { anchor, .. } = write.value {
                    anchors.insert(anchor);
                }
            }
        }
        for anchor in anchors {
            self.settle_family(anchor);
        }
    }

    /// Settles the family `anchor` heads ([`Run::siblings`]).
    fn settle_family(&mut self, anchor: usize) {
        let Some(type_name) = self
            .resources
            .get(anchor)
            .map(|resource| resource.type_name.clone())
        else {
            return;
        };
        let mut members = BTreeMap::from([(1u32, anchor)]);
        for (index, resource) in self.resources.iter().enumerate() {
            if let Identity::Sibling { anchor: own, label } = resource.identity
                && own == anchor
            {
                members.insert(label, index);
            }
        }
        let mut present = BTreeSet::new();
        let mut urls = BTreeMap::new();
        let mut edges = BTreeMap::new();
        for (label, index) in &members {
            let Some(resource) = self.resources.get(*index) else {
                continue;
            };
            urls.insert(*label, resource.full_url.clone());
            if !resource.writes.is_empty() {
                present.insert(*label);
            }
            for write in &resource.writes {
                if let Pending::Sibling { map, .. } = &write.value
                    && let Some(map) = self.corpus.get(map)
                {
                    for (from, to) in sibling_edges(map, &type_name) {
                        edges.entry(from).or_insert(to);
                    }
                }
            }
        }
        let targets = self.link_siblings(&members, &present, &urls, &edges);
        if present.is_empty() {
            return;
        }
        let finest: Vec<u32> = present.difference(&targets).copied().collect();
        let anchor_url = urls.get(&1).cloned().unwrap_or_default();
        let first = self
            .resources
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != anchor)
            .flat_map(|(_, resource)| &resource.writes)
            .find(|write| references(write, &anchor_url))
            .map(|write| (write.at.clone(), write.row.clone()));
        let anchor_present = present.contains(&1);
        let moved_to = if let [level] = finest.as_slice() {
            if let Some((at, row)) = first {
                self.outcomes.push(Outcome::FinestSibling {
                    at,
                    row,
                    label: *level,
                });
            }
            urls.get(level).cloned()
        } else {
            if let Some((at, row)) = first {
                self.outcomes.push(Outcome::SiblingAmbiguous {
                    at,
                    row,
                    labels: finest,
                });
            }
            None
        };
        self.move_references(anchor, &anchor_url, moved_to.as_deref(), anchor_present);
        let anchor_left =
            !anchor_present || moved_to.as_ref().is_some_and(|url| *url != anchor_url);
        for (label, index) in &members {
            if let Some(resource) = self.resources.get_mut(*index)
                && resource.writes.is_empty()
                && (*label != 1 || anchor_left)
            {
                resource.omitted = true;
            }
        }
    }

    /// Settles each reference between the siblings of `members` to the
    /// valued sibling it reaches ([`climb`]), counting one that reaches none
    /// as [`Outcome::SiblingUnresolved`], and returns the siblings reached.
    fn link_siblings(
        &mut self,
        members: &BTreeMap<u32, usize>,
        present: &BTreeSet<u32>,
        urls: &BTreeMap<u32, String>,
        edges: &BTreeMap<u32, u32>,
    ) -> BTreeSet<u32> {
        let mut targets = BTreeSet::new();
        let mut unresolved = Vec::new();
        for (own, index) in members {
            let Some(resource) = self.resources.get_mut(*index) else {
                continue;
            };
            resource.writes.retain_mut(|write| {
                let Pending::Sibling { label, .. } = write.value else {
                    return true;
                };
                let reached = climb(*own, label, present, edges)
                    .and_then(|level| urls.get(&level).map(|url| (level, url.clone())));
                if let Some((level, url)) = reached {
                    targets.insert(level);
                    write.value = Pending::Ready(Value::String(url));
                    return true;
                }
                unresolved.push(Outcome::SiblingUnresolved {
                    at: write.at.clone(),
                    row: write.row.clone(),
                    label,
                });
                false
            });
        }
        self.outcomes.extend(unresolved);
        targets
    }

    /// Moves every reference to the anchor at `anchor_url` to `moved_to`;
    /// with no sibling to move to, the references stay when the anchor holds
    /// values (`kept`) and are dropped when it holds none.
    fn move_references(
        &mut self,
        anchor: usize,
        anchor_url: &str,
        moved_to: Option<&str>,
        kept: bool,
    ) {
        let references_anchor = |write: &Write| references(write, anchor_url);
        for (index, resource) in self.resources.iter_mut().enumerate() {
            if index == anchor {
                continue;
            }
            match moved_to {
                Some(url) => {
                    for write in &mut resource.writes {
                        if references_anchor(write) {
                            write.value = Pending::Ready(Value::String(String::from(url)));
                        }
                    }
                }
                None if !kept => resource.writes.retain(|write| !references_anchor(write)),
                None => {}
            }
        }
    }
}

/// Whether `write` is the `reference` of a `Reference` to the resource at
/// `url`.
fn references(write: &Write, url: &str) -> bool {
    write
        .slots
        .last()
        .is_some_and(|slot| slot.name == "reference")
        && matches!(&write.value, Pending::Ready(Value::String(written)) if written == url)
}

/// Whether a data type map's rows reference instances of `resource_type`
/// by label, as `partOf.reference(Location[2])` in a map into `Location`.
fn names_siblings(map: &Map, resource_type: &str) -> bool {
    !sibling_edges(map, resource_type).is_empty()
}

/// The sibling each `[k].` row family of `map` names by a labelled
/// reference to `resource_type`, the first row's for each `k`.
fn sibling_edges(map: &Map, resource_type: &str) -> BTreeMap<u32, u32> {
    let mut edges = BTreeMap::new();
    for row in &map.rows {
        let Ok(Target::Path {
            instance: Some(Label::Number(from)),
            steps,
        }) = &row.target
        else {
            continue;
        };
        let Some(target) = steps.last().and_then(|step| step.reference.as_ref()) else {
            continue;
        };
        if let Some(Label::Number(to)) = target.label
            && target.resource == resource_type
            && target.path.is_empty()
        {
            edges.entry(*from).or_insert(to);
        }
    }
    edges
}

/// The valued sibling a reference from `own` to `label` reaches, climbing
/// from a sibling no value reached to the one its own row names: `None` for
/// a reference back to `own`, a loop, or a climb that ends unvalued.
fn climb(own: u32, label: u32, present: &BTreeSet<u32>, edges: &BTreeMap<u32, u32>) -> Option<u32> {
    let mut level = label;
    let mut seen = BTreeSet::new();
    loop {
        if level == own || !seen.insert(level) {
            return None;
        }
        if present.contains(&level) {
            return Some(level);
        }
        level = *edges.get(&level)?;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fhir_types::codec::Value;

    use crate::map::Outcome;
    use crate::map::corpus::Corpus;
    use crate::map::run::tests::{corpus, message, shipped};
    use crate::map::run::{Pending, Run};
    use crate::parse::Parsed;

    /// A synthetic ADT^A01 whose PV1-3 is `pl`.
    fn admission(pl: &str) -> Parsed {
        let pv1 = format!("PV1|1|I|{pl}");
        message(&[
            "MSH|^~\\&|ADT|NORTHHOSP|EHR|SOUTHCLINIC|20260926100000+0200||ADT^A01^ADT_A01|MSG00050|P|2.5.1",
            "EVN|A01|20260926100000+0200",
            "PID|1||PAT-0050^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19800101|M",
            &pv1,
        ])
    }

    /// Walks `parsed` and settles its sibling families, as `finish` does
    /// before it applies the writes.
    fn settled<'a>(corpus: &'a Corpus, parsed: &'a Parsed) -> Run<'a> {
        let mut run = Run::new(corpus, parsed);
        run.message().expect("the walk runs");
        run.siblings();
        run
    }

    /// The text a resource's writes put at `path`.
    fn written(resource: &crate::map::run::Resource, path: &str) -> Option<String> {
        resource.writes.iter().find_map(|write| {
            let names: Vec<&str> = write.slots.iter().map(|slot| slot.name.as_str()).collect();
            match &write.value {
                Pending::Ready(Value::String(text)) if names.join(".") == path => {
                    Some(text.clone())
                }
                _ => None,
            }
        })
    }

    /// The levels from the Location the Encounter references up its
    /// `partOf` chain, finest first, as identifier value and physical type.
    fn chain(run: &Run<'_>) -> Vec<(String, String)> {
        let locations: BTreeMap<&str, &crate::map::run::Resource> = run
            .resources
            .iter()
            .filter(|resource| resource.type_name == "Location" && !resource.omitted)
            .map(|resource| (resource.full_url.as_str(), resource))
            .collect();
        let mut next = run
            .resources
            .iter()
            .find(|resource| resource.type_name == "Encounter")
            .and_then(|encounter| written(encounter, "location.location.reference"));
        let mut levels = Vec::new();
        while let Some(url) = next {
            let Some(location) = locations.get(url.as_str()) else {
                break;
            };
            levels.push((
                written(location, "identifier.value").unwrap_or_default(),
                written(location, "physicalType.coding.code").unwrap_or_default(),
            ));
            assert!(levels.len() <= locations.len(), "the chain loops");
            next = written(location, "partOf.reference");
        }
        levels
    }

    /// The Locations of a run that enter the Bundle.
    fn locations<'r>(run: &'r Run<'_>) -> Vec<&'r crate::map::run::Resource> {
        run.resources
            .iter()
            .filter(|resource| resource.type_name == "Location" && !resource.omitted)
            .collect()
    }

    fn levels(levels: &[(&str, &str)]) -> Vec<(String, String)> {
        levels
            .iter()
            .map(|(value, code)| (String::from(*value), String::from(*code)))
            .collect()
    }

    // NOTE: `datatype-pl-to-location` writes one Location per PL level by `[k].`, linked by
    // `partOf` (FHIR R4 Location.partOf); the supplement orders the chain as the PL.10 rows do.
    #[test]
    fn a_pl_with_every_level_gives_one_chain_and_the_encounter_references_the_bed() {
        let corpus = shipped();
        let parsed = admission("WARD4^R12^B2^NORTHHOSP^^^BLDG-A^FL3^North wing bed 2");
        let run = settled(&corpus, &parsed);
        assert_eq!(
            chain(&run),
            levels(&[
                ("B2", "bd"),
                ("R12", "ro"),
                ("WARD4", "wa"),
                ("FL3", "lvl"),
                ("BLDG-A", "bu"),
                ("NORTHHOSP", "si"),
            ]),
            "{:?}",
            run.outcomes
        );
        assert_eq!(locations(&run).len(), 6);
        let described: Vec<String> = locations(&run)
            .into_iter()
            .filter_map(|location| written(location, "description"))
            .collect();
        assert_eq!(described, vec![String::from("North wing bed 2")]);
        assert!(
            run.outcomes
                .iter()
                .any(|outcome| matches!(outcome, Outcome::FinestSibling { label: 1, .. }))
        );
        assert!(
            !run.outcomes.iter().any(|outcome| matches!(
                outcome,
                Outcome::UnplacedInstance { .. } | Outcome::SiblingUnresolved { .. }
            )),
            "{:?}",
            run.outcomes
        );
    }

    // NOTE: no specification governs this: our own design; without PL.3 the Encounter takes the
    // room, and a reference past an unvalued floor and building climbs to the facility.
    #[test]
    fn a_pl_without_a_bed_references_the_room() {
        let corpus = shipped();
        let parsed = admission("WARD4^R12^^NORTHHOSP^^^^^Room 12");
        let run = settled(&corpus, &parsed);
        assert_eq!(
            chain(&run),
            levels(&[("R12", "ro"), ("WARD4", "wa"), ("NORTHHOSP", "si")]),
            "{:?}",
            run.outcomes
        );
        assert_eq!(locations(&run).len(), 3);
        let room = locations(&run)
            .into_iter()
            .find(|location| written(location, "identifier.value").as_deref() == Some("R12"))
            .and_then(|room| written(room, "description"));
        assert_eq!(room.as_deref(), Some("Room 12"));
        assert!(
            run.outcomes
                .iter()
                .any(|outcome| matches!(outcome, Outcome::FinestSibling { label: 2, .. }))
        );
    }

    // NOTE: `datatype-pl-to-location` row PL.7 `[5].partOf.reference(Location[5])` names its own
    // level, so the guide's building is part of nothing and stands beside the bed's chain.
    #[test]
    fn the_guides_building_that_names_itself_leaves_two_finest_levels() {
        let corpus = corpus();
        let parsed = admission("WARD4^R12^B2^NORTHHOSP^^^BLDG-A");
        let run = settled(&corpus, &parsed);
        assert!(
            run.outcomes
                .iter()
                .any(|outcome| matches!(outcome, Outcome::SiblingUnresolved { label: 5, .. }))
        );
        assert!(run.outcomes.iter().any(|outcome| matches!(
            outcome,
            Outcome::SiblingAmbiguous { labels, .. } if *labels == [1, 5]
        )));
        assert_eq!(
            chain(&run).first().map(|(value, _)| value.as_str()),
            Some("B2")
        );
    }

    // NOTE: `mapping_guidelines.md` §\[n\] Notation: a `[k].` prefix on a map rooted at a resource
    // that names no sibling of its own type stays an instance nothing places.
    #[test]
    fn a_map_naming_no_sibling_of_its_own_type_keeps_no_family() {
        let corpus = corpus();
        let map = corpus
            .get("datatype-ndl-to-practitionerrole")
            .expect("the guide ships the NDL map");
        assert!(!super::names_siblings(map, "PractitionerRole"));
        let map = corpus
            .get("datatype-pl-to-location")
            .expect("the guide ships the PL map");
        assert!(super::names_siblings(map, "Location"));
    }
}
