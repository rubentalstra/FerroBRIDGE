// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `MessageHeader` source and destination endpoints.

use ferrobridge_hl7v2::map::{Mapped, Outcome, map};
use fhir_types::codec::{Json, Path, Value};

use crate::fixtures;
use crate::support::{self};

use crate::map::{entries, line, mapped, summary, tables};

/// The outcomes of `mapped` of the kind `kind`, as summary lines.
fn lines_of(mapped: &Mapped, kind: &str) -> Vec<String> {
    mapped
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == kind)
        .map(line)
        .collect()
}

/// The `source` and first `destination` of the Bundle's `MessageHeader`.
fn header_parts(mapped: &Mapped) -> (Value, Value) {
    let header = entries(mapped.bundle())
        .first()
        .and_then(|entry| entry.get("resource"))
        .expect("the Bundle has a first entry");
    assert_eq!(
        header.get("resourceType").and_then(Value::as_str),
        Some("MessageHeader"),
        "HL7 R4 Bundle bdl-12: a message Bundle opens with its MessageHeader"
    );
    let source = header.get("source").cloned().expect("a source");
    let destination = header
        .get("destination")
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .cloned()
        .expect("a destination");
    (source, destination)
}

fn text_at<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value.get(key).and_then(Value::as_str)
}

// NOTE: HL7 R4 datatypes §url admits no whitespace, so an application named only by
// its namespace ID gets the derived endpoint and keeps the name in `name`.
#[tokio::test]
async fn an_application_named_with_spaces_gets_a_derived_endpoint_and_keeps_its_name() {
    let (mapped, _) = mapped(&fixtures::oru_r01_named_applications()).await;
    let (source, destination) = header_parts(&mapped);
    assert_eq!(
        text_at(&source, "endpoint"),
        Some("urn:ferrobridge:hl7v2-hd:North%20Lab%20App")
    );
    assert_eq!(text_at(&source, "name"), Some("North Lab App"));
    assert_eq!(
        text_at(&destination, "endpoint"),
        Some("urn:ferrobridge:hl7v2-hd:South%20EHR")
    );
    assert_eq!(text_at(&destination, "name"), Some("South EHR"));
    assert!(
        lines_of(&mapped, "invalid-value").is_empty(),
        "{:#?}",
        summary(&mapped)
    );
    assert!(
        lines_of(&mapped, "missing-required")
            .iter()
            .all(|line| !line.contains("MessageHeader")),
        "{:#?}",
        summary(&mapped)
    );
    let object = mapped.bundle().as_object().expect("a Bundle object");
    fhir_types::r4::bundle::Bundle::from_json(object, &mut Path::root("Bundle"))
        .expect("the Bundle decodes as R4");
}

// NOTE: the guide's `datatype-hd-endpoint-to-messageheader-source` map writes an ISO
// universal ID as `urn:oid:` and a UUID as `urn:uuid:` before HD.2.
#[tokio::test]
async fn a_typed_universal_id_takes_the_guide_row_where_one_runs_else_the_derived_form() {
    let (mapped, _) = mapped(&fixtures::oru_r01_universal_applications()).await;
    let (source, destination) = header_parts(&mapped);
    // NOTE: `datatype-hd-endpoint-to-messageheader-source`, the HD.2 ISO row: MSH-3's `source[1]`
    // row runs it, so the guide's `urn:oid:` form wins over the bridge's fallback.
    assert_eq!(text_at(&source, "endpoint"), Some("urn:oid:1.2.3.4.5"));
    assert_eq!(text_at(&source, "name"), Some("LAB"));
    // NOTE: `segment-msh-to-messageheader`, the MSH-5 endpoint row: no data type map fills an HD
    // into a `url`, so the endpoint is the bridge's derived form of every component.
    assert_eq!(
        text_at(&destination, "endpoint"),
        Some("urn:ferrobridge:hl7v2-hd:EHR:2b3c4d5e-0000-4000-8000-000000000001:UUID")
    );
    assert_eq!(text_at(&destination, "name"), Some("EHR"));
    assert!(
        lines_of(&mapped, "components-dropped")
            .iter()
            .all(|line| !line.contains("MSH[1]-3") && !line.contains("MSH[1]-5"))
    );
    let object = mapped.bundle().as_object().expect("a Bundle object");
    fhir_types::r4::bundle::Bundle::from_json(object, &mut Path::root("Bundle"))
        .expect("the Bundle decodes as R4");
}

// NOTE: no specification governs this: our own design; with MSH-3 and MSH-24 empty the
// sending facility gives the source endpoint, and MSH-6 the destination's, each counted.
#[tokio::test]
async fn a_facility_alone_gives_the_endpoint_and_the_fallback_is_counted() {
    let (mapped, _) = mapped(&fixtures::oru_r01_facilities_only()).await;
    let (source, destination) = header_parts(&mapped);
    assert_eq!(
        text_at(&source, "endpoint"),
        Some("urn:ferrobridge:hl7v2-hd:North%20Lab:1.2.3.4.5:ISO")
    );
    assert_eq!(text_at(&source, "name"), Some("North Lab"));
    assert_eq!(
        text_at(&destination, "endpoint"),
        Some("urn:ferrobridge:hl7v2-hd:SOUTHCLINIC")
    );
    assert_eq!(text_at(&destination, "name"), Some("SOUTHCLINIC"));
    let fallbacks: Vec<(String, String)> = mapped
        .outcomes()
        .iter()
        .filter_map(|outcome| match outcome {
            Outcome::FacilityEndpoint { at, row, element } => {
                Some((format!("{at} {}", row.source), element.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        fallbacks,
        vec![
            (
                String::from("MSH[1]-4(1) MSH-4"),
                String::from("MessageHeader.source.endpoint")
            ),
            (
                String::from("MSH[1]-6(1) MSH-6"),
                String::from("MessageHeader.destination.endpoint")
            ),
        ],
        "{:#?}",
        summary(&mapped)
    );
    let header = entries(mapped.bundle())
        .first()
        .and_then(|entry| entry.get("resource"))
        .cloned()
        .expect("a MessageHeader");
    assert!(
        header
            .get("sender")
            .and_then(|sender| sender.get("reference"))
            .is_some(),
        "the guide's MSH-4 row keeps its sender Organization: {header:?}"
    );
    let object = mapped.bundle().as_object().expect("a Bundle object");
    fhir_types::r4::bundle::Bundle::from_json(object, &mut Path::root("Bundle"))
        .expect("the Bundle decodes as R4");
}

#[tokio::test]
async fn a_valued_application_takes_no_facility_fallback() {
    let (mapped, _) = mapped(&fixtures::oru_r01_named_applications()).await;
    assert!(
        lines_of(&mapped, "facility-endpoint").is_empty(),
        "{:#?}",
        summary(&mapped)
    );
}

// NOTE: HL7 R4 JSON §Primitive Types: the guide's MSH-24 row writes the data-absent-reason as
// `_endpoint` alone, which is the required endpoint, so a message naming no sender maps.
#[tokio::test]
async fn a_message_naming_no_sender_keeps_the_data_absent_reason_endpoint() {
    let (mapped, _) = mapped(&fixtures::oru_r01_without_sender()).await;
    let (source, _) = header_parts(&mapped);
    assert_eq!(source.get("endpoint"), None, "{source:?}");
    let extension = source
        .get("_endpoint")
        .and_then(|endpoint| endpoint.get("extension"))
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .cloned()
        .expect("a data-absent-reason extension");
    assert_eq!(
        extension.get("valueCode").and_then(Value::as_str),
        Some("unknown"),
        "{extension:?}"
    );
    assert!(
        lines_of(&mapped, "facility-endpoint")
            .iter()
            .all(|line| !line.contains("MSH-4")),
        "{:#?}",
        summary(&mapped)
    );
    let object = mapped.bundle().as_object().expect("a Bundle object");
    fhir_types::r4::bundle::Bundle::from_json(object, &mut Path::root("Bundle"))
        .expect("the Bundle decodes as R4");
}

// NOTE: `datatype-hd-endpoint-to-messageheader-source`, the HD.2 rows: an HD.3 of ISO,
// UUID, DNS or URI gives the endpoint `urn:<type>:` and HD.2, and the Bundle keeps bdl-12.
#[tokio::test]
async fn the_hd_endpoint_maps_write_the_guides_urn_endpoint_per_universal_id_type() {
    for (hd_type, universal, source_endpoint, destination_endpoint) in [
        ("ISO", "1.2.3.9", "urn:oid:1.2.3.9", "urn:oid:1.2.3.9"),
        (
            "UUID",
            "0b7e4a2c-58f1-4c3e-9d2a-6f1e8b3c4d5a",
            "urn:uuid:0b7e4a2c-58f1-4c3e-9d2a-6f1e8b3c4d5a",
            "urn:uuid:0b7e4a2c-58f1-4c3e-9d2a-6f1e8b3c4d5a",
        ),
        (
            "DNS",
            "lab.example.org",
            "urn:dns:lab.example.org",
            "lab.example.org",
        ),
        (
            "URI",
            "http://lab.example.org/v2",
            "urn:uri:http://lab.example.org/v2",
            "http://lab.example.org/v2",
        ),
    ] {
        // NOTE: `datatype-hd-name-to-messageheader-destination` writes HD.2 into the `name` its
        // endpoint twin fills from HD.1, so a destination HD.1 is the conflict tested below.
        let source_hd = format!("NORTHNET^{universal}^{hd_type}");
        let destination_hd = format!("^{universal}^{hd_type}");
        let fixture = fixtures::oru_r01_network_addresses(&source_hd, &destination_hd);
        let (mapped, _) = mapped(&fixture).await;
        let object = mapped.bundle().as_object().expect("a Bundle object");
        fhir_types::r4::bundle::Bundle::from_json(object, &mut Path::root("Bundle"))
            .expect("the Bundle decodes as R4");
        let (source, destination) = header_parts(&mapped);
        assert_eq!(text_at(&source, "name"), Some("NORTHNET"), "{source:?}");
        assert_eq!(
            text_at(&source, "endpoint"),
            Some(source_endpoint),
            "{hd_type}: {source:?}"
        );
        // NOTE: `datatype-hd-endpoint-to-messageheader-destination` writes the `urn:` form for
        // ISO and UUID only, and HD.2 as it stands for any other type.
        assert_eq!(
            text_at(&destination, "endpoint"),
            Some(destination_endpoint),
            "{hd_type}: {destination:?}"
        );
        assert!(
            mapped.outcomes().iter().all(
                |outcome| !matches!(outcome, Outcome::DatatypeConflict { row, .. }
                    if row.map == "segment-msh-to-messageheader")
            ),
            "{hd_type}: {:?}",
            summary(&mapped)
        );
    }
}

// NOTE: `datatype-hd-endpoint-to-messageheader-destination` writes HD.1 into `name` and
// `datatype-hd-name-to-messageheader-destination` writes HD.2 there, so the values differ.
#[tokio::test]
async fn a_destination_hd_with_a_namespace_id_is_a_counted_conflict_of_the_guides_maps() {
    let fixture =
        fixtures::oru_r01_network_addresses("NORTHNET^1.2.3.9^ISO", "SOUTHNET^1.2.3.10^ISO");
    let (mapped, _) = mapped(&fixture).await;
    let conflicts: Vec<(String, Vec<String>)> = mapped
        .outcomes()
        .iter()
        .filter_map(|outcome| match outcome {
            Outcome::DatatypeConflict {
                row, element, maps, ..
            } if row.source == "MSH-25" => Some((element.clone(), maps.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        conflicts,
        vec![(
            String::from("name"),
            vec![
                String::from("datatype-hd-endpoint-to-messageheader-destination"),
                String::from("datatype-hd-name-to-messageheader-destination"),
            ]
        )],
        "{:#?}",
        summary(&mapped)
    );
    let source = entries(mapped.bundle())
        .first()
        .and_then(|entry| entry.get("resource"))
        .and_then(|header| header.get("source"))
        .cloned()
        .expect("a MessageHeader source");
    assert_eq!(text_at(&source, "endpoint"), Some("urn:oid:1.2.3.9"));
    assert_eq!(text_at(&source, "name"), Some("NORTHNET"));
}

// NOTE: `datatype-hd-endpoint-to-messageheader-source`, the HD.3 rows `IF NOT VALUED OR NOT IN
// (...)`: an HD.3 outside the four types gives the endpoint a data-absent-reason extension.
#[tokio::test]
async fn a_universal_id_of_another_type_gives_the_guides_data_absent_endpoint() {
    let fixture = fixtures::oru_r01_network_addresses("^LAB-7^L", "^1.2.3.10^ISO");
    let (mapped, _) = mapped(&fixture).await;
    let (source, _) = header_parts(&mapped);
    assert_eq!(source.get("endpoint"), None, "{source:?}");
    let extension = source
        .get("_endpoint")
        .and_then(|endpoint| endpoint.get("extension"))
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .cloned()
        .expect("a data-absent-reason extension on the endpoint");
    assert_eq!(
        text_at(&extension, "url"),
        Some("http://hl7.org/fhir/R4/extension-data-absent-reason.html")
    );
    assert_eq!(text_at(&extension, "valueCode"), Some("unknown"));
    assert_eq!(text_at(&source, "name"), Some(" - L:LAB-7"));
}

#[tokio::test]
async fn a_network_address_of_a_url_yielding_type_completes_the_source() {
    let (mapped, _) = mapped(&fixtures::oru_r01_network_address_only()).await;
    let (source, _) = header_parts(&mapped);
    assert_eq!(
        text_at(&source, "endpoint"),
        Some("urn:oid:1.2.3.9"),
        "{source:?}"
    );
}

// NOTE: HL7 R4 Bundle bdl-12: a message Bundle opens with a MessageHeader, and
// MessageHeader.source is 1..1, so a message whose source no row completes maps to no Bundle.
#[tokio::test]
async fn a_message_whose_source_no_row_completes_maps_to_no_bundle() {
    // NOTE: `datatype-hd-endpoint-to-messageheader-source`: an HD of a namespace ID alone meets
    // no endpoint row, since the HD.3 rows write only on a valued HD.3 outside the four types.
    let parsed = support::parsed(&fixtures::oru_r01_network_addresses(
        "NORTHNET",
        "^1.2.3.10^ISO",
    ));
    let corpus = support::corpus();
    let tables = tables();
    let (_server, client) = tables.serve().await;
    let refused = map(&parsed, &corpus, Some(&client)).await;
    match refused {
        Err(ferrobridge_hl7v2::map::MapError::NoMessageHeader { dropped }) => {
            assert!(
                matches!(
                    dropped.as_deref(),
                    Some(Outcome::MissingRequired { required, .. })
                        if required == "MessageHeader.source"
                ),
                "{dropped:?}"
            );
        }
        other => panic!("the map should refuse the message: {other:?}"),
    }
}

// NOTE: `segment-msh-to-messageheader`, the MSH-24 valueCode row's comment: the implementer
// assigns a known value or the data-absent-reason, so a facility endpoint carries no extension.
#[tokio::test]
async fn a_facility_endpoint_carries_no_data_absent_reason() {
    let (mapped, _) = mapped(&fixtures::oru_r01_facilities_only()).await;
    let (source, destination) = header_parts(&mapped);
    assert_eq!(source.get("_endpoint"), None, "{source:?}");
    assert_eq!(destination.get("_endpoint"), None, "{destination:?}");
}
