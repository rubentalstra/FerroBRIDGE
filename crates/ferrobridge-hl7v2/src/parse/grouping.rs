// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The grouping of a lexed message by its structure.

use hl7v2_types::model::{Node, SegmentRef, Structure};

use crate::parse::lex::Lexed;
use crate::parse::place::close;
use crate::parse::place::{Missing, place};
use crate::parse::required::check_fields;
use crate::parse::{ErrorCode, Frame, Item, Parsed, Refusal, Unplaced, base_name};

/// Whether `nodes` place the segment `id` anywhere in their tree.
fn places(nodes: &'static [Node], id: &str) -> bool {
    nodes.iter().any(|node| match node {
        Node::Segment(reference) => reference.segment.id == id,
        Node::Group(group) => places(group.children, id),
        Node::Placeholder(_) => false,
    })
}

/// Groups `lexed` by `structure`.
///
/// Each segment is placed at the next node of the structure that can hold it,
/// searching forward from the node the previous segment filled: a later
/// sibling, a new instance of a repeating group whose first segments include
/// it, or a sibling of an enclosing group. A segment with no such node is
/// counted as [`Unplaced::OutOfStructure`], and the cursor stays where it was.
/// A required segment or group left empty is a refusal. A required field
/// left empty in a placed segment is counted as
/// [`Unplaced::MissingRequiredField`], and refuses only for MSH-9, MSH-10 and
/// MSH-12.
#[must_use]
pub fn group(lexed: Lexed, structure: &'static Structure) -> Parsed {
    let Lexed { message, refusals } = lexed;
    let mut parsed = Parsed {
        message,
        structure,
        items: Vec::new(),
        unplaced: Vec::new(),
        refusals,
    };
    if let Some(withdrawn_as_of) = structure.withdrawn_as_of {
        parsed.unplaced.push(Unplaced::WithdrawnStructure {
            structure: structure.id,
            version: structure.version,
            withdrawn_as_of,
            declared: parsed.message.version().map(String::from),
        });
    } else {
        let declared = parsed.message.version();
        if structure.version != crate::DEFINITIONS_VERSION
            || declared != Some(crate::DEFINITIONS_VERSION)
        {
            parsed.unplaced.push(Unplaced::VersionSelected {
                structure: structure.id,
                version: structure.version,
                declared: declared.map(String::from),
            });
        }
        if structure.version == crate::DEFINITIONS_VERSION
            && let Some(declared) = declared
            && declared != crate::DEFINITIONS_VERSION
        {
            parsed.unplaced.push(Unplaced::EarlierVersion {
                declared: String::from(declared),
            });
        }
    }
    if let Some(declared) = parsed.message.message_type(3)
        && declared != base_name(structure.id)
    {
        parsed.unplaced.push(Unplaced::OtherStructure {
            declared: String::from(declared),
            structure: structure.id,
        });
    }
    let mut stack = vec![Frame::root(structure.nodes)];
    let mut missing = Vec::new();
    let routed: Vec<(usize, &str)> = parsed
        .message
        .segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| {
            hl7v2_types::segment::find(&segment.id).is_some()
                || places(structure.nodes, &segment.id)
        })
        .map(|(index, segment)| (index, segment.id.as_str()))
        .collect();
    let ids: Vec<&str> = routed.iter().map(|(_, id)| *id).collect();
    let mut next = 0;
    for index in 0..parsed.message.segments.len() {
        let location = parsed.location(index);
        let Some(&(_, id)) = routed.get(next).filter(|(at, _)| *at == index) else {
            parsed.unplaced.push(Unplaced::UnknownSegment { location });
            continue;
        };
        next = next.saturating_add(1);
        let rest = ids.get(next..).unwrap_or_default();
        if !place(&mut stack, id, index, rest, &mut missing) {
            parsed.unplaced.push(Unplaced::OutOfStructure { location });
        }
    }
    while stack.len() > 1 {
        close(&mut stack, &mut missing);
    }
    if let Some(root) = stack.pop() {
        root.check(&mut missing);
        parsed.items = root.items;
    }
    for Missing { detail, at } in missing {
        parsed.refusals.push(Refusal {
            location: at.map(|index| parsed.location(index)).unwrap_or_default(),
            code: ErrorCode::SegmentSequence,
            detail: format!("the required {detail} is missing"),
        });
    }
    let placed = placed_segments(&parsed.items);
    for (index, node) in placed {
        check_fields(&mut parsed, index, node);
    }
    parsed
}

/// Every placed segment with the node it fills, in message order.
fn placed_segments(items: &[Item]) -> Vec<(usize, &'static SegmentRef)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Segment(placed) => out.push((placed.index, placed.node)),
            Item::Group(instance) => out.extend(placed_segments(&instance.items)),
        }
    }
    out.sort_by_key(|(index, _)| *index);
    out
}
