// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The placement of one segment in the segment-group tree: the open group
//! instances and the node that can hold the segment next.

use hl7v2_types::model::{Group, GroupKind, Max, Node};

use crate::parse::{Frame, Instance, Item, Placed};

impl Frame {
    /// The frame of the structure's top level.
    pub(super) fn root(nodes: &'static [Node]) -> Self {
        Self {
            nodes,
            group: None,
            occurrence: 0,
            at: 0,
            counts: vec![0; nodes.len()],
            items: Vec::new(),
        }
    }

    /// The frame of a new instance of `group`.
    fn instance(group: &'static Group, occurrence: usize) -> Self {
        Self {
            nodes: group.children,
            group: Some(group),
            occurrence,
            at: 0,
            counts: vec![0; group.children.len()],
            items: Vec::new(),
        }
    }

    /// A copy of the frame's cursor without its items, for a trial walk.
    fn shadow(&self) -> Self {
        Self {
            nodes: self.nodes,
            group: self.group,
            occurrence: self.occurrence,
            at: self.at,
            counts: self.counts.clone(),
            items: Vec::new(),
        }
    }

    /// Whether this frame is a choice group whose alternative is already
    /// taken, so only that alternative can take more segments.
    fn locked(&self) -> bool {
        self.is_choice() && self.counts.iter().any(|count| *count > 0)
    }

    /// Whether this frame is an instance of a choice group.
    fn is_choice(&self) -> bool {
        self.group
            .is_some_and(|group| group.kind == GroupKind::Choice)
    }

    /// Records a required node left empty, naming the group that lacks it.
    pub(super) fn check(&self, missing: &mut Vec<Missing>) {
        if self.is_choice() {
            return;
        }
        for (node, count) in self.nodes.iter().zip(&self.counts) {
            if *count > 0 {
                continue;
            }
            let member = match node {
                Node::Segment(reference) if reference.cardinality.min > 0 => {
                    format!("segment {}", reference.id)
                }
                Node::Group(group) if group.cardinality.min > 0 => format!("group {}", group.id),
                Node::Segment(_) | Node::Group(_) | Node::Placeholder(_) => continue,
            };
            let detail = match self.group {
                Some(group) => format!("{member} of group {}", group.name),
                None => member,
            };
            missing.push(Missing {
                detail,
                at: first_index(&self.items),
            });
        }
    }
}

/// A required member no segment filled.
#[derive(Debug)]
pub(super) struct Missing {
    /// The member, and the group lacking it when that is not the
    /// structure's top level.
    pub(super) detail: String,
    /// The index of the first segment of the group instance lacking it,
    /// `None` when the instance holds no segment.
    pub(super) at: Option<usize>,
}

/// The index of the first segment in `items`.
fn first_index(items: &[Item]) -> Option<usize> {
    items.first().and_then(|item| match item {
        Item::Segment(placed) => Some(placed.index),
        Item::Group(instance) => first_index(&instance.items),
    })
}

/// Whether one more occurrence fits under `max` after `count`.
fn fits(count: usize, max: Max) -> bool {
    match max {
        Max::Unbounded => true,
        Max::Bounded(bound) => usize::try_from(bound).map_or(true, |bound| count < bound),
    }
}

/// Whether `node` must occur at least once.
fn required(node: Option<&Node>) -> bool {
    match node {
        Some(Node::Segment(reference)) => reference.cardinality.min > 0,
        Some(Node::Group(group)) => group.cardinality.min > 0,
        Some(Node::Placeholder(_)) | None => false,
    }
}

/// Places the segment `id` at `index`, returning whether a node took it.
///
/// `rest` holds the ids of the segments that follow, in message order. The
/// first node that accepts the segment, searching the innermost open group
/// first, takes it, unless it opens a group the following segments leave
/// with a required member empty: then the first later node whose taking
/// leaves no required member empty takes it, and the first node does when
/// no such node exists.
// NOTE: the HL7/v2ig message structures (`message_structures/*.json`) give each group member its
// `min`; choosing among the nodes that accept a segment by those cardinalities is our own design.
pub(super) fn place(
    stack: &mut Vec<Frame>,
    id: &str,
    index: usize,
    rest: &[&str],
    missing: &mut Vec<Missing>,
) -> bool {
    let Some(first) = accepting(stack, id) else {
        return false;
    };
    let chosen = if completes(stack, first, id, index, rest) {
        first
    } else {
        alternatives(stack, id)
            .into_iter()
            .filter(|found| *found != first)
            .find(|found| clean(stack, *found, id, index, rest))
            .unwrap_or(first)
    };
    take(stack, chosen, id, index, missing)
}

/// Places the segment `id` at the first node that accepts it, without a
/// look at the segments that follow.
fn place_first(stack: &mut Vec<Frame>, id: &str, index: usize, missing: &mut Vec<Missing>) {
    if let Some(found) = accepting(stack, id) {
        take(stack, found, id, index, missing);
    }
}

/// Closes the frames above `depth` and takes node `position` of the frame
/// at `depth`.
fn take(
    stack: &mut Vec<Frame>,
    (depth, position): (usize, usize),
    id: &str,
    index: usize,
    missing: &mut Vec<Missing>,
) -> bool {
    while stack.len() > depth.saturating_add(1) {
        close(stack, missing);
    }
    enter(stack, position, id, index)
}

/// The depth and node of the first node that accepts `id`, innermost open
/// group first.
fn accepting(stack: &[Frame], id: &str) -> Option<(usize, usize)> {
    stack
        .iter()
        .enumerate()
        .rev()
        .find_map(|(depth, frame)| candidate(frame, id, frame.at).map(|position| (depth, position)))
}

/// Every node that accepts `id`, innermost open group first and in
/// definition order within a group.
fn alternatives(stack: &[Frame], id: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (depth, frame) in stack.iter().enumerate().rev() {
        let mut from = frame.at;
        while let Some(position) = candidate(frame, id, from) {
            out.push((depth, position));
            from = position.saturating_add(1);
        }
    }
    out
}

/// Whether taking `found` for `id` leaves no required member empty: none
/// in the groups it closes, none among the nodes it passes over, and none
/// in the group it opens.
fn clean(stack: &[Frame], found: (usize, usize), id: &str, index: usize, rest: &[&str]) -> bool {
    let (depth, position) = found;
    let mut missing = Vec::new();
    for frame in stack.iter().skip(depth.saturating_add(1)) {
        frame.check(&mut missing);
    }
    let Some(frame) = stack.get(depth) else {
        return false;
    };
    let passes = !frame.is_choice()
        && (frame.at..position).any(|skipped| {
            frame.counts.get(skipped).copied().unwrap_or_default() == 0
                && required(frame.nodes.get(skipped))
        });
    missing.is_empty() && !passes && completes(stack, found, id, index, rest)
}

/// Whether the group `found` opens, when it is a group, has every required
/// member filled by the time it closes: a trial walk of the segments in
/// `rest` over a copy of the open groups, each placed at the first node
/// that accepts it, until one falls outside the group.
fn completes(
    stack: &[Frame],
    found: (usize, usize),
    id: &str,
    index: usize,
    rest: &[&str],
) -> bool {
    let (depth, position) = found;
    let opens = stack
        .get(depth)
        .and_then(|frame| frame.nodes.get(position))
        .is_some_and(|node| matches!(node, Node::Group(_)));
    if !opens {
        return true;
    }
    let mut trial: Vec<Frame> = stack
        .iter()
        .take(depth.saturating_add(1))
        .map(Frame::shadow)
        .collect();
    if !enter(&mut trial, position, id, index) {
        return false;
    }
    let opened = depth.saturating_add(1);
    let mut missing = Vec::new();
    for next in rest {
        match accepting(&trial, next) {
            Some((at, _)) if at < opened => break,
            Some(_) => place_first(&mut trial, next, index, &mut missing),
            None => {}
        }
    }
    while trial.len() > opened {
        close(&mut trial, &mut missing);
    }
    missing.is_empty()
}

/// The node of `frame` from node `from` on that can take `id`.
fn candidate(frame: &Frame, id: &str, from: usize) -> Option<usize> {
    let last = if frame.locked() {
        frame.at
    } else {
        frame.nodes.len().saturating_sub(1)
    };
    (from..=last).find(|position| {
        let count = frame.counts.get(*position).copied().unwrap_or_default();
        match frame.nodes.get(*position) {
            Some(Node::Segment(reference)) => {
                reference.segment.id == id && fits(count, reference.cardinality.max)
            }
            Some(Node::Group(group)) => {
                fits(count, group.cardinality.max) && starts(&group_first(group), id)
            }
            Some(Node::Placeholder(_)) | None => false,
        }
    })
}

/// Takes node `position` of the top frame for the segment, entering groups
/// down to the segment node.
fn enter(stack: &mut Vec<Frame>, position: usize, id: &str, index: usize) -> bool {
    let Some(frame) = stack.last_mut() else {
        return false;
    };
    frame.at = position;
    let occurrence = frame.counts.get(position).copied().unwrap_or_default();
    if let Some(count) = frame.counts.get_mut(position) {
        *count = count.saturating_add(1);
    }
    match frame.nodes.get(position) {
        Some(Node::Segment(reference)) => {
            frame.items.push(Item::Segment(Placed {
                index,
                node: reference,
                occurrence,
            }));
            true
        }
        Some(Node::Group(group)) => {
            stack.push(Frame::instance(group, occurrence));
            let Some(inner) = stack
                .last()
                .and_then(|frame| candidate(frame, id, frame.at))
            else {
                return false;
            };
            enter(stack, inner, id, index)
        }
        Some(Node::Placeholder(_)) | None => false,
    }
}

/// Closes the top frame into its parent.
pub(super) fn close(stack: &mut Vec<Frame>, missing: &mut Vec<Missing>) {
    let Some(frame) = stack.pop() else {
        return;
    };
    frame.check(missing);
    let Some(group) = frame.group else {
        return;
    };
    if let Some(parent) = stack.last_mut() {
        parent.items.push(Item::Group(Instance {
            group,
            occurrence: frame.occurrence,
            items: frame.items,
        }));
    }
}

/// Whether `first` holds `id`.
fn starts(first: &[&'static str], id: &str) -> bool {
    first.contains(&id)
}

/// The segment ids a new instance of `group` can open with.
fn group_first(group: &'static Group) -> Vec<&'static str> {
    let mut first = Vec::new();
    for child in group.children {
        let (ids, required) = match child {
            Node::Segment(reference) => (vec![reference.segment.id], reference.cardinality.min > 0),
            Node::Group(inner) => (group_first(inner), inner.cardinality.min > 0),
            Node::Placeholder(_) => (Vec::new(), false),
        };
        first.extend(ids);
        if required && group.kind == GroupKind::Sequence {
            break;
        }
    }
    first
}
