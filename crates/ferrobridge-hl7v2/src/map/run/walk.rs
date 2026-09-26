// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The walk over the placed segments, in message order, through the
//! message map.

use std::collections::BTreeSet;

use hl7v2_types::model::Max;

use crate::map::corpus::{Map, Origin};
use crate::map::notation::Target;
use crate::map::run::segment::row_ref;
use crate::map::run::{Base, Run, Scope, Visit};
use crate::map::{MapError, Outcome};
use crate::parse::{Item, Location};

impl<'a> Run<'a> {
    /// Counts `map` as [`Outcome::Supplemented`] at `at` when it is a
    /// supplement the run has not counted yet.
    ///
    /// No specification governs this: our own design; once per map and
    /// message keeps the count a record of which supplements a message used.
    pub(super) fn supplemented(&mut self, map: &Map, at: &Location) {
        let Origin::Supplement { overrides } = map.origin else {
            return;
        };
        let counted = self.outcomes.iter().any(
            |outcome| matches!(outcome, Outcome::Supplemented { map: id, .. } if *id == map.id),
        );
        if !counted {
            self.outcomes.push(Outcome::Supplemented {
                at: at.clone(),
                map: map.id.clone(),
                overrides,
            });
        }
    }

    /// Maps one placed segment through the message map rows naming it.
    pub(super) fn visit(&mut self, map: &'a Map, visit: &Visit<'a>) -> Result<(), MapError> {
        let at = self.parsed.location(visit.index);
        let Some(rows) = self.rows(map, visit, &at) else {
            self.outcomes.push(Outcome::UnmappedSegment { at });
            return Ok(());
        };
        self.supplemented(map, &at);
        let scope = Scope {
            parsed: self.parsed,
            index: visit.index,
            definition: visit.definition,
            chain: visit.chain.clone(),
            at: at.clone(),
            repetition: None,
            datatype: None,
        };
        let Some(segment) = scope.segment() else {
            return Ok(());
        };
        let mut named = BTreeSet::new();
        let mut ran = false;
        for row in rows {
            let reference = row_ref(map, row);
            if !self.gate(&scope, row, &reference)? {
                continue;
            }
            let Ok(Target::Path { steps, .. }) = &row.target else {
                if let Err(error) = &row.target {
                    self.outcomes.push(Outcome::UnsupportedTarget {
                        at: at.clone(),
                        row: reference,
                        error: error.clone(),
                    });
                }
                continue;
            };
            let Some((head, rest)) = steps.split_first() else {
                continue;
            };
            let target_type = rest
                .last()
                .and_then(|step| step.reference.as_ref())
                .map_or(head.name.as_str(), |reference| reference.resource.as_str());
            let segment_map = match self.corpus.find("segment", segment.id(), target_type) {
                Ok(found) => found,
                Err(candidates) => {
                    self.outcomes.push(Outcome::NoSegmentMap {
                        at: at.clone(),
                        resource: String::from(target_type),
                        candidates,
                    });
                    continue;
                }
            };
            let resource = if head.name == "Bundle" {
                0
            } else {
                self.message_resource(&head.name, head.label.clone(), &visit.key)
            };
            let base = Base {
                resource,
                slots: Vec::new(),
            };
            let base = match rest.iter().position(|step| step.reference.is_some()) {
                Some(position) if position.saturating_add(1) == rest.len() => {
                    let referencing = rest.get(..=position).unwrap_or_default();
                    let Some(created) = self.reference(&base, referencing, 0, &at, &reference)
                    else {
                        continue;
                    };
                    created
                }
                Some(_) => {
                    self.outcomes.push(Outcome::UnsupportedShape {
                        at: at.clone(),
                        row: reference,
                    });
                    continue;
                }
                // NOTE: `mapping_guidelines.md` §Message Spreadsheet: a sub-path such as
                // `Observation[2].note` repeats what the segment map's resource-rooted rows
                // already write, so the segment map runs at the resource root.
                None => base,
            };
            ran = true;
            self.segment(&scope, segment_map, &base, &mut named)?;
            self.facility_endpoints(&scope, segment_map, &base);
        }
        if ran {
            self.unmapped_fields(&scope, &named);
        }
        Ok(())
    }
}

/// Collects every placed segment in message order.
pub(super) fn collect<'p>(
    items: &'p [Item],
    structure: &str,
    path: &mut Vec<&'static str>,
    key: &mut Vec<usize>,
    chain: &mut Vec<&'p [Item]>,
    top: &'static [hl7v2_types::model::Node],
    visits: &mut Vec<Visit<'p>>,
) {
    chain.insert(0, items);
    for item in items {
        match item {
            Item::Segment(placed) => {
                let mut segment_key = key.clone();
                if repeats(placed.node.cardinality.max) {
                    segment_key.push(placed.occurrence);
                }
                let mut code = String::from(structure);
                for name in path.iter() {
                    code.push('.');
                    code.push_str(name);
                }
                code.push('.');
                code.push_str(placed.node.segment.id);
                let top_follow = if path.is_empty() {
                    preceding(top, placed.node.id, placed.node.segment.id)
                } else {
                    Vec::new()
                };
                visits.push(Visit {
                    index: placed.index,
                    definition: placed.node.segment,
                    code,
                    top_follow,
                    key: segment_key,
                    chain: chain.clone(),
                });
            }
            Item::Group(instance) => {
                path.push(instance.group.name);
                let pushed = repeats(instance.group.cardinality.max);
                if pushed {
                    key.push(instance.occurrence);
                }
                collect(&instance.items, structure, path, key, chain, top, visits);
                if pushed {
                    key.pop();
                }
                path.pop();
            }
        }
    }
    chain.remove(0);
}

/// The ids of the top-level segments before the node `id`, back to the
/// previous node of the same segment id.
fn preceding(
    nodes: &'static [hl7v2_types::model::Node],
    id: &str,
    segment: &str,
) -> Vec<&'static str> {
    let Some(position) = nodes.iter().position(
        |node| matches!(node, hl7v2_types::model::Node::Segment(reference) if reference.id == id),
    ) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for node in nodes.get(..position).unwrap_or_default().iter().rev() {
        if let hl7v2_types::model::Node::Segment(reference) = node {
            if reference.segment.id == segment {
                break;
            }
            ids.push(reference.segment.id);
        }
    }
    ids
}

/// Whether a node repeats.
pub(super) const fn repeats(max: Max) -> bool {
    match max {
        Max::Unbounded => true,
        Max::Bounded(bound) => bound > 1,
    }
}
