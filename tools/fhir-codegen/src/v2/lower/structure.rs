// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Message structures: the dotted element path read as the group tree.

use std::collections::{BTreeMap, BTreeSet};

use crate::roots::V2_SEGMENT_BASE;
use crate::v2::corpus::{Corpus, Sourced};
use crate::v2::definition::Element;
use crate::v2::lower::BACKBONE;
use crate::v2::lower::Cardinality;
use crate::v2::lower::DEFINITIONS_VERSION;
use crate::v2::lower::Defect;
use crate::v2::lower::GroupKind;
use crate::v2::lower::Input;
use crate::v2::lower::LowerError;
use crate::v2::lower::Node;
use crate::v2::lower::PLACEHOLDER_GROUP_NAME;
use crate::v2::lower::SEGMENT_STATUS;
use crate::v2::lower::SegmentStatus;
use crate::v2::lower::Structure;
use crate::v2::lower::defect;
use crate::v2::lower::extension::value_code;
use crate::v2::lower::invalid;
use crate::v2::lower::segment::cardinality;
use crate::v2::lower::segment::elements;
use crate::v2::lower::segment::position;
use crate::v2::lower::segment::single_type;

/// One step of a structure element id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step<'a> {
    /// `n-NAME`.
    Numbered(u16, &'a str),
    /// `choice-n-NAME`.
    Choice(u16, &'a str),
    /// `segment`, the single-segment member of an `Hxx` slot.
    SlotSegment,
    /// `group`, the nested-group member of an `Hxx` slot.
    SlotGroup,
}

fn parse_step<'a>(
    sourced: &Input<'_>,
    element: &str,
    step: &'a str,
) -> Result<Step<'a>, LowerError> {
    match step {
        "segment" => return Ok(Step::SlotSegment),
        "group" => return Ok(Step::SlotGroup),
        _ => {}
    }
    let (choice, rest) = match step.strip_prefix("choice-") {
        Some(rest) => (true, rest),
        None => (false, step),
    };
    let Some((digits, name)) = rest.split_once('-') else {
        return Err(invalid(
            sourced,
            element,
            format!("step {step:?} is not n-NAME"),
        ));
    };
    if name.is_empty() {
        return Err(invalid(
            sourced,
            element,
            format!("step {step:?} has no name"),
        ));
    }
    let position = position(sourced, element, digits)?;
    Ok(if choice {
        Step::Choice(position, name)
    } else {
        Step::Numbered(position, name)
    })
}

/// The children of each element id, in definition order.
type Children<'a> = BTreeMap<&'a str, Vec<&'a Element>>;

pub(super) fn lower_structure<'a>(
    corpus: &'a Corpus,
    sourced: &Input<'_>,
    reached: &mut BTreeMap<String, &'a Sourced>,
) -> Result<Structure, LowerError> {
    let elements = elements(sourced, crate::roots::V2_STRUCTURE_BASE)?;
    let id = sourced.definition.id.as_str();
    let mut children: Children<'_> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    seen.insert(id);
    for element in elements.iter().skip(1) {
        let Some((parent, _)) = element.id.rsplit_once('.') else {
            return Err(invalid(
                sourced,
                &element.id,
                "an element outside the structure",
            ));
        };
        if !seen.contains(parent) {
            return Err(invalid(
                sourced,
                &element.id,
                format!("its parent {parent} does not come before it"),
            ));
        }
        if !seen.insert(element.id.as_str()) {
            return Err(invalid(sourced, &element.id, "the id is given twice"));
        }
        children.entry(parent).or_default().push(element);
    }
    let nodes = if children.is_empty() {
        defect(sourced, id, Defect::EmptyStructure)?;
        Vec::new()
    } else {
        let (kind, nodes) = lower_children(corpus, sourced, &children, id, reached)?;
        if kind != GroupKind::Sequence {
            return Err(invalid(sourced, id, "the structure root is a choice"));
        }
        nodes
    };
    Ok(Structure {
        id: id.to_owned(),
        url: Some(sourced.definition.url.clone()),
        version: String::from(DEFINITIONS_VERSION),
        withdrawn_as_of: None,
        nodes,
    })
}

fn lower_children<'a>(
    corpus: &'a Corpus,
    sourced: &Input<'_>,
    children: &Children<'_>,
    parent: &str,
    reached: &mut BTreeMap<String, &'a Sourced>,
) -> Result<(GroupKind, Vec<Node>), LowerError> {
    let Some(elements) = children.get(parent) else {
        return Err(invalid(
            sourced,
            parent,
            "a structure or group with no children",
        ));
    };
    let mut kind = None;
    let mut nodes = Vec::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
        let last = element
            .id
            .rsplit_once('.')
            .map_or(element.id.as_str(), |(_, last)| last);
        let (step_kind, position, name) = match parse_step(sourced, &element.id, last)? {
            Step::Numbered(position, name) => (GroupKind::Sequence, position, name),
            Step::Choice(position, name) => (GroupKind::Choice, position, name),
            Step::SlotSegment | Step::SlotGroup => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    "an Hxx member outside an Hxx slot",
                ));
            }
        };
        if *kind.get_or_insert(step_kind) != step_kind {
            return Err(invalid(
                sourced,
                &element.id,
                "sequence and choice steps mixed among siblings",
            ));
        }
        if usize::from(position) != index + 1 {
            return Err(invalid(
                sourced,
                &element.id,
                format!("position {position} where {} belongs", index + 1),
            ));
        }
        nodes.push(lower_node(
            corpus, sourced, children, element, position, name, reached,
        )?);
    }
    Ok((kind.unwrap_or(GroupKind::Sequence), nodes))
}

fn lower_node<'a>(
    corpus: &'a Corpus,
    sourced: &Input<'_>,
    children: &Children<'_>,
    element: &Element,
    position: u16,
    name: &str,
    reached: &mut BTreeMap<String, &'a Sourced>,
) -> Result<Node, LowerError> {
    if element.content_reference.is_some() {
        return Err(invalid(
            sourced,
            &element.id,
            "a contentReference outside an Hxx slot",
        ));
    }
    let cardinality = cardinality(sourced, element)?;
    let status = segment_status(sourced, element)?;
    let Some(code) = single_type(sourced, element)? else {
        return Err(invalid(
            sourced,
            &element.id,
            "an untyped structure element",
        ));
    };
    if code == BACKBONE {
        if status.is_some() {
            return Err(invalid(
                sourced,
                &element.id,
                "a group carries a segment status",
            ));
        }
        if let Some(slot) = placeholder(sourced, children, element)? {
            return Ok(Node::Placeholder {
                id: element.id.clone(),
                position,
                cardinality: slot,
            });
        }
        if name == PLACEHOLDER_GROUP_NAME {
            defect(sourced, &element.id, Defect::PlaceholderGroupName)?;
        }
        let (kind, nodes) = lower_children(corpus, sourced, children, &element.id, reached)?;
        return Ok(Node::Group {
            id: element.id.clone(),
            position,
            name: name.to_owned(),
            cardinality,
            kind,
            children: nodes,
        });
    }
    if children.contains_key(element.id.as_str()) {
        return Err(invalid(sourced, &element.id, "a segment with children"));
    }
    let Some(target) = corpus.segments().get(code) else {
        return Err(invalid(
            sourced,
            &element.id,
            format!("type {code} is no segment definition"),
        ));
    };
    let segment = target.definition.id.clone();
    if segment != name {
        return Err(invalid(
            sourced,
            &element.id,
            format!("step name {name} names another segment than {code}"),
        ));
    }
    reached.insert(segment.clone(), target);
    Ok(Node::Segment {
        id: element.id.clone(),
        position,
        segment,
        cardinality,
        status,
    })
}

/// The cardinality of `element` when it is an `Hxx` slot: a group whose
/// children are the `segment` and `group` members.
fn placeholder(
    sourced: &Input<'_>,
    children: &Children<'_>,
    element: &Element,
) -> Result<Option<Cardinality>, LowerError> {
    let Some(members) = children.get(element.id.as_str()) else {
        return Ok(None);
    };
    let steps: Vec<&str> = members
        .iter()
        .map(|member| member.id.rsplit_once('.').map_or("", |(_, last)| last))
        .collect();
    if !steps
        .iter()
        .any(|step| *step == "segment" || *step == "group")
    {
        return Ok(None);
    }
    let name = element.id.rsplit_once('.').map_or("", |(_, last)| last);
    let named = name.ends_with("-Hxx");
    let [segment, group] = members.as_slice() else {
        return Err(invalid(
            sourced,
            &element.id,
            "an Hxx slot is exactly a segment and a group member",
        ));
    };
    let one = Cardinality {
        min: 0,
        max: Some(1),
    };
    let segment_ok = steps.first() == Some(&"segment")
        && single_type(sourced, segment)? == Some(V2_SEGMENT_BASE)
        && cardinality(sourced, segment)? == one
        && segment.extension.is_empty()
        && !children.contains_key(segment.id.as_str());
    let group_ok = steps.get(1) == Some(&"group")
        && group.types.is_none()
        && group.content_reference.as_deref() == Some(format!("#{}", element.id).as_str())
        && cardinality(sourced, group)? == one
        && group.extension.is_empty()
        && !children.contains_key(group.id.as_str());
    if !(named && segment_ok && group_ok) {
        return Err(invalid(
            sourced,
            &element.id,
            "an Hxx slot is exactly a segment and a group member",
        ));
    }
    cardinality(sourced, element).map(Some)
}

fn segment_status(
    sourced: &Input<'_>,
    element: &Element,
) -> Result<Option<SegmentStatus>, LowerError> {
    let mut status = None;
    for extension in &element.extension {
        if extension.url != SEGMENT_STATUS {
            return Err(invalid(
                sourced,
                &element.id,
                format!("unknown extension {}", extension.url),
            ));
        }
        if status.is_some() {
            return Err(invalid(sourced, &element.id, "segment status given twice"));
        }
        let code = value_code(sourced, element, extension)?;
        status = Some(match code {
            "A" => Some(SegmentStatus::A),
            "B" => Some(SegmentStatus::B),
            "D" => Some(SegmentStatus::D),
            "d" => {
                defect(sourced, &element.id, Defect::LowercaseStatus)?;
                Some(SegmentStatus::D)
            }
            "" => {
                defect(sourced, &element.id, Defect::EmptyStatus)?;
                None
            }
            other => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("segment status {other:?}"),
                ));
            }
        });
    }
    Ok(status.flatten())
}
