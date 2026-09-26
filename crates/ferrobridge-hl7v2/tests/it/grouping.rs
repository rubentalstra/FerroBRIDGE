// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The choice among the nodes that accept a segment, by the cardinalities
//! of the group members the `HL7/v2ig` message structures give.

use ferrobridge_hl7v2::parse::{ErrorCode, Item, Parsed};

use crate::{fixtures, support};

/// The group names from the top level down to the placed segment at
/// message index `index`, `None` when no node holds it.
fn groups_of(parsed: &Parsed, index: usize) -> Option<Vec<&'static str>> {
    fn walk(items: &[Item], index: usize, path: &mut Vec<&'static str>) -> bool {
        for item in items {
            match item {
                Item::Segment(placed) if placed.index == index => return true,
                Item::Segment(_) => {}
                Item::Group(instance) => {
                    path.push(instance.group.name);
                    if walk(&instance.items, index, path) {
                        return true;
                    }
                    path.pop();
                }
            }
        }
        false
    }
    let mut path = Vec::new();
    walk(parsed.items(), index, &mut path).then_some(path)
}

/// The refusals as code, ERL location and detail.
fn refusals(parsed: &Parsed) -> Vec<(ErrorCode, String, String)> {
    parsed
        .refusals()
        .iter()
        .map(|refusal| {
            (
                refusal.code,
                refusal.location.erl('^'),
                refusal.detail.clone(),
            )
        })
        .collect()
}

/// An OUL^R22 from a 2.9.1 sender: one specimen, its order and the order's
/// common order, then one result.
fn oul_r22_with_orc() -> Vec<u8> {
    fixtures::message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260926101500+0200||OUL^R22^OUL_R22|MSG00343|P|2.9.1",
        b"SPM|1|SPC-343||BLD^Whole blood^HL70487",
        b"OBR|1|PLC-343|FIL-343|2345-7^Glucose^LN",
        b"ORC|SC|PLC-343|FIL-343",
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|3.9-5.8|N|||F",
    ])
}

#[test]
fn an_obx_after_the_common_order_goes_to_the_result_group_when_no_txa_follows() {
    let parsed = support::parsed(&oul_r22_with_orc());
    assert_eq!(
        groups_of(&parsed, 4),
        Some(vec!["SPECIMEN", "ORDER", "RESULT"]),
        "the OBX opens a RESULT, since ORDER_DOCUMENT requires a TXA none follows"
    );
    assert_eq!(
        groups_of(&parsed, 3),
        Some(vec!["SPECIMEN", "ORDER", "COMMON_ORDER"])
    );
    assert_eq!(refusals(&parsed), []);
}

#[test]
fn an_obx_followed_by_its_txa_opens_the_order_document() {
    let mut bytes = oul_r22_with_orc();
    bytes.extend(fixtures::message(&[
        b"TXA|1|CN|TX|20260926100000+0200||||||||DOC-0343|||||AU",
    ]));
    let parsed = support::parsed(&bytes);
    assert_eq!(
        groups_of(&parsed, 4),
        Some(vec!["SPECIMEN", "ORDER", "COMMON_ORDER", "ORDER_DOCUMENT"])
    );
    assert_eq!(
        groups_of(&parsed, 5),
        Some(vec!["SPECIMEN", "ORDER", "COMMON_ORDER", "ORDER_DOCUMENT"])
    );
    assert_eq!(refusals(&parsed), []);
}

#[test]
fn a_group_left_with_a_required_member_missing_is_a_refusal_naming_both() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260926101500+0200||OUL^R22^OUL_R22|MSG00344|P|2.9.1",
        b"SPM|1|SPC-344||BLD^Whole blood^HL70487",
    ]);
    let parsed = support::parsed(&bytes);
    assert_eq!(groups_of(&parsed, 1), Some(vec!["SPECIMEN"]));
    assert_eq!(
        refusals(&parsed),
        [(
            ErrorCode::SegmentSequence,
            String::from("SPM^1"),
            String::from(
                "the required group OUL_R22.8-SPECIMEN.4-ORDER of group SPECIMEN is missing"
            ),
        )]
    );
}
