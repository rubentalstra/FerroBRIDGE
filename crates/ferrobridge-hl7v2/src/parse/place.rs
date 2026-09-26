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

    /// Whether this frame is a choice group whose alternative is already
    /// taken, so only that alternative can take more segments.
    fn locked(&self) -> bool {
        self.group
            .is_some_and(|group| group.kind == GroupKind::Choice)
            && self.counts.iter().any(|count| *count > 0)
    }

    /// Records a required node left empty.
    pub(super) fn check(&self, missing: &mut Vec<String>) {
        if self
            .group
            .is_some_and(|group| group.kind == GroupKind::Choice)
        {
            return;
        }
        for (node, count) in self.nodes.iter().zip(&self.counts) {
            if *count > 0 {
                continue;
            }
            match node {
                Node::Segment(reference) if reference.cardinality.min > 0 => {
                    missing.push(format!("segment {}", reference.id));
                }
                Node::Group(group) if group.cardinality.min > 0 => {
                    missing.push(format!("group {}", group.id));
                }
                Node::Segment(_) | Node::Group(_) | Node::Placeholder(_) => {}
            }
        }
    }
}

/// Whether one more occurrence fits under `max` after `count`.
fn fits(count: usize, max: Max) -> bool {
    match max {
        Max::Unbounded => true,
        Max::Bounded(bound) => usize::try_from(bound).map_or(true, |bound| count < bound),
    }
}

/// Places the segment `id` at `index`, returning whether a node took it.
pub(super) fn place(
    stack: &mut Vec<Frame>,
    id: &str,
    index: usize,
    missing: &mut Vec<String>,
) -> bool {
    for depth in (0..stack.len()).rev() {
        let Some(frame) = stack.get(depth) else {
            continue;
        };
        let Some(found) = candidate(frame, id) else {
            continue;
        };
        while stack.len() > depth.saturating_add(1) {
            close(stack, missing);
        }
        return enter(stack, found, id, index);
    }
    false
}

/// The node of `frame` that can take `id`, searching from the current one.
fn candidate(frame: &Frame, id: &str) -> Option<usize> {
    let last = if frame.locked() {
        frame.at
    } else {
        frame.nodes.len().saturating_sub(1)
    };
    (frame.at..=last).find(|position| {
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
            let Some(inner) = stack.last().and_then(|frame| candidate(frame, id)) else {
                return false;
            };
            enter(stack, inner, id, index)
        }
        Some(Node::Placeholder(_)) | None => false,
    }
}

/// Closes the top frame into its parent.
pub(super) fn close(stack: &mut Vec<Frame>, missing: &mut Vec<String>) {
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
