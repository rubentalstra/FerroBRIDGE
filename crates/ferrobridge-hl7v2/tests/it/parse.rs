// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Positional parsing and the grouping by the generated message structures.

use std::collections::BTreeMap;

use ferrobridge_hl7v2::parse::{ErrorCode, Item, Parsed, StructureError, Unplaced, structure_for};

use crate::{fixtures, support};

/// Renders the group tree as indented lines of group names and segment ids.
fn tree(parsed: &Parsed) -> String {
    fn walk(parsed: &Parsed, items: &[Item], depth: usize, out: &mut String) {
        for item in items {
            match item {
                Item::Segment(placed) => {
                    let id = parsed.message().segments()[placed.index].id();
                    out.push_str(&"  ".repeat(depth));
                    out.push_str(id);
                    out.push('\n');
                }
                Item::Group(instance) => {
                    out.push_str(&"  ".repeat(depth));
                    out.push_str(instance.group.name);
                    out.push('[');
                    out.push_str(&instance.occurrence.to_string());
                    out.push_str("]\n");
                    walk(parsed, &instance.items, depth + 1, out);
                }
            }
        }
    }
    let mut out = String::new();
    walk(parsed, parsed.items(), 0, &mut out);
    out
}

#[test]
fn an_oru_r01_is_grouped_by_its_structure() {
    let parsed = support::parsed(&fixtures::oru_r01());
    assert_eq!(parsed.structure().id, "ORU_R01-A");
    assert_eq!(
        tree(&parsed),
        "MSH\n\
         PATIENT_RESULT[0]\n\
         \x20 PATIENT[0]\n\
         \x20   PID\n\
         \x20   VISIT[0]\n\
         \x20     PV1\n\
         \x20 ORDER_OBSERVATION[0]\n\
         \x20   COMMON_ORDER[0]\n\
         \x20     ORC\n\
         \x20   OBR\n\
         \x20   OBSERVATION[0]\n\
         \x20     OBX\n\
         \x20   OBSERVATION[1]\n\
         \x20     OBX\n"
    );
    assert!(parsed.refusals().is_empty(), "{:?}", parsed.refusals());
}

#[test]
fn a_z_segment_is_a_counted_outcome() {
    let parsed = support::parsed(&fixtures::oru_r01());
    let unknown: Vec<String> = parsed
        .unplaced()
        .iter()
        .filter_map(|outcome| match outcome {
            Unplaced::UnknownSegment { location } => Some(location.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(unknown, ["ZXX[1]"]);
}

#[test]
fn a_2_5_1_sender_is_parsed_under_the_later_definitions_and_counted() {
    let parsed = support::parsed(&fixtures::oru_r01());
    assert!(parsed.unplaced().contains(&Unplaced::EarlierVersion {
        declared: String::from("2.5.1"),
    }));
}

#[test]
fn an_adt_a01_is_grouped_by_its_structure() {
    let parsed = support::parsed(&fixtures::adt_a01());
    assert_eq!(parsed.structure().id, "ADT_A01-A");
    assert_eq!(tree(&parsed), "MSH\nEVN\nPID\nPV1\n");
    assert!(parsed.refusals().is_empty(), "{:?}", parsed.refusals());
}

#[test]
fn a_missing_required_field_is_a_refusal_at_its_location() {
    let parsed = support::parsed(&fixtures::oru_r01_without_patient_name());
    let refusals: Vec<(String, ErrorCode)> = parsed
        .refusals()
        .iter()
        .map(|refusal| (refusal.location.erl('^'), refusal.code))
        .collect();
    assert_eq!(
        refusals,
        [(String::from("PID^1^5"), ErrorCode::RequiredFieldMissing)]
    );
}

#[test]
fn a_missing_required_segment_is_a_segment_sequence_refusal() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|ADMIT|NORTHHOSP|EHR|SOUTHCLINIC|20260925090000+0200||ADT^A01^ADT_A01|MSG1|P|2.5.1",
        b"EVN||20260925085900+0200",
        b"PID|1||PAT-0002^^^NORTHHOSP^MR||Doe^Sam",
    ]);
    let parsed = support::parsed(&bytes);
    assert_eq!(
        parsed
            .refusals()
            .iter()
            .map(|refusal| (refusal.code, refusal.detail.as_str()))
            .collect::<Vec<_>>(),
        [(
            ErrorCode::SegmentSequence,
            "the required segment ADT_A01-A.19-PV1 is missing"
        )]
    );
}

#[test]
fn a_known_segment_out_of_its_place_is_counted() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|ADMIT|NORTHHOSP|EHR|SOUTHCLINIC|20260925090000+0200||ADT^A01^ADT_A01|MSG1|P|2.5.1",
        b"EVN||20260925085900+0200",
        b"PID|1||PAT-0002^^^NORTHHOSP^MR||Doe^Sam",
        b"PV1|1|I",
        b"MSA|AA|MSG0",
    ]);
    let parsed = support::parsed(&bytes);
    assert!(parsed.unplaced().iter().any(|outcome| matches!(
        outcome,
        Unplaced::OutOfStructure { location } if location.segment == "MSA"
    )));
}

#[test]
fn a_valued_field_beyond_the_segment_table_is_counted() {
    let mut fields = String::from("PV1|1|I");
    fields.push_str(&"|".repeat(60));
    fields.push_str("EXTRA");
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|ADMIT|NORTHHOSP|EHR|SOUTHCLINIC|20260925090000+0200||ADT^A01^ADT_A01|MSG1|P|2.5.1",
        b"EVN||20260925085900+0200",
        b"PID|1||PAT-0002^^^NORTHHOSP^MR||Doe^Sam",
        fields.as_bytes(),
    ]);
    let parsed = support::parsed(&bytes);
    assert!(parsed.unplaced().iter().any(|outcome| matches!(
        outcome,
        Unplaced::ExtraField { location } if location.segment == "PV1"
    )));
}

/// Lexes `bytes` as ASCII and selects its structure.
fn select(bytes: &[u8]) -> Result<&'static str, StructureError> {
    let decoded =
        ferrobridge_hl7v2::decode::decode(bytes, ferrobridge_hl7v2::decode::Charset::Ascii)
            .expect("the message decodes");
    let lexed = ferrobridge_hl7v2::parse::lex(&decoded.text, decoded.charset).expect("it lexes");
    structure_for(&lexed.message).map(|structure| structure.id)
}

/// A synthetic header with `message_type` as MSH-9, and an EVN, a PID and a PV1.
fn with_type(message_type: &str) -> Vec<u8> {
    let header = format!(
        "MSH|^~\\&|ADMIT|NORTHHOSP|EHR|SOUTHCLINIC|20260925090000+0200||{message_type}|MSG00009|P|2.5.1"
    );
    fixtures::message(&[
        header.as_bytes(),
        b"EVN||20260925085900+0200",
        b"PID|1||PAT-0009^^^NORTHHOSP^MR||Test^Anna^^^^^L||19920304|F",
        b"PV1|1|O",
    ])
}

#[test]
fn the_message_definition_of_the_trigger_event_selects_the_variant() {
    // HL7/v2ig input/sourceOfTruth/message/messages/ORU-R01.json names ORU_R01-A.
    assert_eq!(select(&fixtures::oru_r01()), Ok("ORU_R01-A"));
}

#[test]
fn a_variant_other_than_the_first_is_chosen_from_the_message_table() {
    // HL7/v2ig input/sourceOfTruth/message/messages/ADT-A04.json names ADT_A01-B.
    assert_eq!(select(&with_type("ADT^A04^ADT_A01")), Ok("ADT_A01-B"));
}

#[test]
fn a_structure_in_variants_no_message_definition_names_is_refused() {
    assert!(matches!(
        select(&with_type("ORU^Z99^ORU_R01")),
        Err(StructureError::Variant { ref candidates, .. })
            if candidates == &["ORU_R01-A", "ORU_R01-B", "ORU_R01-C", "ORU_R01-D"]
    ));
}

// NOTE: HL7 v2.5.1 chapter 2 §2.15.9.9; HL7/v2ig input/sourceOfTruth/message/messages/
// ADT-A08.json names ADT_A01-C, which answers for the ADT_A08 no definition carries.
#[test]
fn an_unknown_structure_falls_back_to_the_message_definition_and_is_counted() {
    let bytes = with_type("ADT^A08^ADT_A08");
    assert_eq!(select(&bytes), Ok("ADT_A01-C"));
    let parsed = support::parsed(&bytes);
    assert!(
        parsed.unplaced().contains(&Unplaced::OtherStructure {
            declared: String::from("ADT_A08"),
            structure: "ADT_A01-C",
        }),
        "{:?}",
        parsed.unplaced()
    );
    assert!(parsed.refusals().is_empty(), "{:?}", parsed.refusals());
}

#[test]
fn a_structure_named_as_declared_counts_no_other_structure() {
    let parsed = support::parsed(&with_type("ADT^A04^ADT_A01"));
    assert!(
        parsed
            .unplaced()
            .iter()
            .all(|outcome| !matches!(outcome, Unplaced::OtherStructure { .. })),
        "{:?}",
        parsed.unplaced()
    );
}

#[test]
fn an_unknown_structure_with_no_message_definition_is_refused() {
    // ORM_O01 has no v2.9.1 message definition but the legacy tables carry it
    // (#303), so the refusal needs a structure neither source carries.
    assert!(matches!(
        select(&with_type("ORM^O01^ORM_Z01")),
        Err(StructureError::Unknown { ref name }) if name == "ORM_Z01"
    ));
    assert_eq!(select(&with_type("ORM^O01^ORM_O01")), Ok("ORM_O01"));
}

#[test]
fn a_known_structure_contradicting_the_message_definition_is_refused() {
    // ADT^A08 names ADT_A01-C, which is no variant of the ADT_A05 MSH-9.3 names.
    assert!(matches!(
        select(&with_type("ADT^A08^ADT_A05")),
        Err(StructureError::Variant { .. })
    ));
}

#[test]
fn a_message_definition_naming_a_variant_of_another_structure_is_refused() {
    // ADT^A04 names ADT_A01-B, which is no variant of the ADT_A05 MSH-9.3 names.
    assert!(matches!(
        select(&with_type("ADT^A04^ADT_A05")),
        Err(StructureError::Variant { .. })
    ));
}

/// The vendored ORM^O01 messages that decode, by the path under `vendor/` and
/// the version their MSH-12 declares.
const ORM_O01_MESSAGES: [(&str, &str); 11] = [
    ("fhir-converter/data/SampleData/Hl7v2/LAB-ORM-1.hl7", "2.3"),
    (
        "reportstream/prime-router/src/testIntegration/resources/datatests/HL7_to_FHIR/sample_orm_20230809-001.hl7",
        "2.5.1",
    ),
    (
        "reportstream/prime-router/src/testIntegration/resources/datatests/mappinginventory/catchall/ormo01/orm_o01-full.hl7",
        "2.5.1",
    ),
    (
        "reportstream/prime-router/src/testIntegration/resources/datatests/mappinginventory/catchall/orcobr/orm-obr-to-specimen.hl7",
        "2.5.1",
    ),
    ("fhir-converter/data/SampleData/Hl7v2/ORM-O01-01.hl7", "2.6"),
    ("fhir-converter/data/SampleData/Hl7v2/ORM-O01-02.hl7", "2.6"),
    ("fhir-converter/data/SampleData/Hl7v2/ORM-O01-03.hl7", "2.6"),
    ("fhir-converter/data/SampleData/Hl7v2/ORM-O01-04.hl7", "2.6"),
    ("fhir-converter/data/SampleData/Hl7v2/ORM-O01-05.hl7", "2.6"),
    ("fhir-converter/data/SampleData/Hl7v2/ORM-O01-06.hl7", "2.6"),
    ("fhir-converter/data/SampleData/Hl7v2/OML-O21-02.hl7", "2.6"),
];

/// The file at `path` under `vendor/` as a frame carries it: the byte order
/// mark removed and every line ending a carriage return.
fn vendored(path: &str) -> Vec<u8> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join(path),
    )
    .expect("the vendored message reads");
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let mut out = Vec::new();
    for line in text.split(|byte| *byte == b'\r' || *byte == b'\n') {
        if !line.iter().all(u8::is_ascii_whitespace) {
            out.extend_from_slice(line);
            out.push(b'\r');
        }
    }
    out
}

#[test]
fn the_vendored_orm_o01_messages_parse_against_the_tree_of_their_version() {
    // Each refusal is a field the tables of the declared version make required
    // (ORC-7 at 2.3, AL1-1 at 2.6) and the message leaves empty.
    let refused: BTreeMap<&str, Vec<&str>> = BTreeMap::from([
        (ORM_O01_MESSAGES[0].0, vec!["ORC[1]-7 ORC.7 is required"]),
        (
            ORM_O01_MESSAGES[3].0,
            vec![
                "MSH[1]-11 MSH.11 is required",
                "PID[1]-3 PID.3 is required",
                "PID[1]-5 PID.5 is required",
                "OBR[1]-4 OBR.4 is required",
            ],
        ),
        (ORM_O01_MESSAGES[5].0, vec!["AL1[1]-1 AL1.1 is required"]),
    ]);
    for (path, version) in ORM_O01_MESSAGES {
        let parsed = support::parsed(&vendored(path));
        assert_eq!(parsed.structure().id, "ORM_O01", "{path}");
        assert_eq!(parsed.structure().version, version, "{path}");
        assert_eq!(parsed.structure().url, None, "{path}");
        let refusals: Vec<String> = parsed
            .refusals()
            .iter()
            .map(|refusal| format!("{} {}", refusal.location, refusal.detail))
            .collect();
        let expected = refused.get(path).cloned().unwrap_or_default();
        assert_eq!(refusals, expected, "{path}");
    }
}

#[test]
fn the_vendored_2_3_1_orm_o01_is_refused_before_selection_for_its_character_set() {
    // MSH-18 declares ASCII and the message carries a byte outside it.
    let bytes = vendored("v2-to-fhir/derived/ORM_O01.hl7");
    assert!(matches!(
        ferrobridge_hl7v2::decode::decode(&bytes, ferrobridge_hl7v2::decode::Charset::Ascii),
        Err(ferrobridge_hl7v2::decode::DecodeError::Undeclared { .. })
    ));
}

#[test]
fn a_withdrawn_structure_is_a_counted_outcome_naming_its_versions() {
    let parsed = support::parsed(&vendored(ORM_O01_MESSAGES[4].0));
    // The IGAMT export lists ORM_O01 at 2.3 to 2.6 and not at 2.7.
    assert!(parsed.unplaced().contains(&Unplaced::WithdrawnStructure {
        structure: "ORM_O01",
        version: "2.6",
        withdrawn_as_of: "2.7",
        declared: Some(String::from("2.6")),
    }));
    assert!(
        !parsed
            .unplaced()
            .iter()
            .any(|outcome| matches!(outcome, Unplaced::EarlierVersion { .. })),
        "a legacy tree is its own version's, so the message is not parsed against 2.9.1"
    );
    assert_eq!(
        parsed
            .unplaced()
            .iter()
            .filter(|outcome| outcome.kind() == "withdrawn-structure")
            .count(),
        1
    );
}

/// A synthetic ORM^O01 at `version`, MSH-9 as `message_type`.
fn orm(message_type: &str, version: &str) -> Vec<u8> {
    let header = format!(
        "MSH|^~\\&|ORDERS|NORTHHOSP|LAB|SOUTHLAB|20260925090000+0200||{message_type}|MSG00031|P|{version}"
    );
    fixtures::message(&[
        header.as_bytes(),
        b"PID|1||PAT-0031^^^NORTHHOSP^MR||Test^Bram||19800101|M",
        b"ORC|NW|PLC-0031",
        b"OBR|1|PLC-0031||GLU^Glucose^L",
    ])
}

#[test]
fn a_withdrawn_structure_is_selected_by_msh_9_3_and_the_declared_version() {
    let selected = |bytes: &[u8]| {
        let decoded =
            ferrobridge_hl7v2::decode::decode(bytes, ferrobridge_hl7v2::decode::Charset::Ascii)
                .expect("the message decodes");
        let lexed =
            ferrobridge_hl7v2::parse::lex(&decoded.text, decoded.charset).expect("it lexes");
        structure_for(&lexed.message).map(|structure| (structure.id, structure.version))
    };
    assert_eq!(
        selected(&orm("ORM^O01^ORM_O01", "2.4")),
        Ok(("ORM_O01", "2.4"))
    );
    assert_eq!(selected(&orm("ORM^O01", "2.3.1")), Ok(("ORM_O01", "2.3.1")));
}

#[test]
fn a_version_the_tables_lack_takes_the_nearest_earlier_tree() {
    // The export carries ORM_O01 up to 2.6, so a 2.8 sender is parsed against 2.6.
    let parsed = support::parsed(&orm("ORM^O01^ORM_O01", "2.8"));
    assert_eq!(parsed.structure().version, "2.6");
    assert!(parsed.unplaced().contains(&Unplaced::WithdrawnStructure {
        structure: "ORM_O01",
        version: "2.6",
        withdrawn_as_of: "2.7",
        declared: Some(String::from("2.8")),
    }));
}

#[test]
fn a_withdrawn_structure_before_its_first_tree_is_refused_with_the_versions() {
    assert_eq!(
        select(&orm("ORM^O01^ORM_O01", "2.2")),
        Err(StructureError::NoLegacyVersion {
            name: String::from("ORM_O01"),
            declared: Some(String::from("2.2")),
            versions: vec!["2.3", "2.3.1", "2.4", "2.5", "2.5.1", "2.6"],
        })
    );
    assert!(matches!(
        select(&orm("ORM^O01", "2.x")),
        Err(StructureError::NoLegacyVersion { .. })
    ));
}

#[test]
fn a_structure_neither_the_definitions_nor_the_legacy_tables_carry_is_unknown() {
    assert_eq!(
        select(&orm("ORM^O01^ORM_Z99", "2.5.1")),
        Err(StructureError::Unknown {
            name: String::from("ORM_Z99"),
        })
    );
}
