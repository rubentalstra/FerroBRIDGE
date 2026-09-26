// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The field pass over each placed segment: a valued field beyond the
//! segment's table, and an empty required field.

use hl7v2_types::model::{Optionality, SegmentRef};

use crate::parse::{ErrorCode, Field, Parsed, Refusal, Unplaced};

/// Refuses a missing required field and counts a valued field beyond the
/// segment's table.
/// The MSH fields whose absence still refuses the message: the message type
/// (MSH-9), the control id (MSH-10) and the version (MSH-12), without which
/// no structure is selected and no acknowledgment names the message. MSH-1
/// and MSH-2 refuse at the lexer.
const ANSWER_FIELDS: [usize; 3] = [9, 10, 12];

/// Checks the fields of the segment at `index` against the definition its
/// node places: an empty required field is counted as
/// [`Unplaced::MissingRequiredField`], one of [`ANSWER_FIELDS`] refuses, and
/// a valued field beyond the table is counted as [`Unplaced::ExtraField`].
pub(super) fn check_fields(parsed: &mut Parsed, index: usize, node: &'static SegmentRef) {
    let location = parsed.location(index);
    let Some(segment) = parsed.message.segments.get(index) else {
        return;
    };
    let definition = node.segment;
    for field in definition.fields {
        let position = usize::from(field.position);
        if field.optionality != Optionality::R
            || segment.field(position).is_some_and(Field::is_valued)
        {
            continue;
        }
        let at = location.clone().with_field(position);
        // NOTE: HL7 table 0516 (hl7.terminology CodeSystem-v2-0516) lets a receiver answer `W`,
        // "transaction successful, but there may be issues"; keeping only the fields the bridge
        // needs to answer at all as refusals is our own design.
        if definition.id == "MSH" && ANSWER_FIELDS.contains(&position) {
            parsed.refusals.push(Refusal {
                location: at,
                code: ErrorCode::RequiredFieldMissing,
                detail: format!("{} is required", field.id),
            });
        } else {
            parsed.unplaced.push(Unplaced::MissingRequiredField {
                location: at,
                field: field.id,
                segment: definition.id,
                version: parsed.structure.version,
            });
        }
    }
    for (offset, field) in segment
        .fields
        .iter()
        .enumerate()
        .skip(definition.fields.len())
    {
        if field.is_valued() {
            parsed.unplaced.push(Unplaced::ExtraField {
                location: location.clone().with_field(offset.saturating_add(1)),
            });
        }
    }
}
