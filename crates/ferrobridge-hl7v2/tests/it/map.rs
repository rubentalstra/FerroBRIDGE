// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The v2-to-FHIR `ConceptMap` interpreter over the vendored
//! `hl7.fhir.uv.v2mappings` 1.0.0 package, with the table maps answered by a
//! stubbed terminology server.

use std::collections::BTreeMap;

use ferrobridge_hl7v2::map::corpus::{Condition, Kind};
use ferrobridge_hl7v2::map::{Mapped, Outcome, map};
use fhir_types::codec::{Json, Path, Value};

use crate::fixtures;
use crate::support::{self, Tables};

/// The table answers a server loaded with the guide's table maps gives for
/// the codes the fixtures carry, read from those maps; the displays come
/// from the target code systems.
pub(crate) fn tables() -> Tables {
    Tables::default()
        .answer(
            "table-hl70004-to-v3-actcode",
            "O",
            "http://terminology.hl7.org/CodeSystem/v3-ActCode",
            "AMB",
            "ambulatory",
        )
        .answer(
            "table-hl70004-to-v3-actcode",
            "I",
            "http://terminology.hl7.org/CodeSystem/v3-ActCode",
            "IMP",
            "inpatient encounter",
        )
        .answer(
            "table-hl70004-to-encounter-status",
            "O",
            "http://hl7.org/fhir/encounter-status",
            "in-progress",
            "In Progress",
        )
        .answer(
            "table-hl70004-to-encounter-status",
            "I",
            "http://hl7.org/fhir/encounter-status",
            "in-progress",
            "In Progress",
        )
        .answer(
            "table-hl70001-to-administrative-gender",
            "U",
            "http://hl7.org/fhir/administrative-gender",
            "unknown",
            "Unknown",
        )
        .answer(
            "table-hl70001-to-administrative-gender",
            "M",
            "http://hl7.org/fhir/administrative-gender",
            "male",
            "Male",
        )
        .answer(
            "table-hl70001-to-administrative-gender",
            "F",
            "http://hl7.org/fhir/administrative-gender",
            "female",
            "Female",
        )
        .answer(
            "table-hl70085-to-observation-status",
            "F",
            "http://hl7.org/fhir/observation-status",
            "final",
            "Final",
        )
        .answer(
            "table-hl70078-to-v3-observationinterpretation",
            "N",
            "http://terminology.hl7.org/CodeSystem/v3-ObservationInterpretation",
            "N",
            "Normal",
        )
        .answer(
            "table-hl70200-to-name-use",
            "L",
            "http://hl7.org/fhir/name-use",
            "official",
            "Official",
        )
        .answer(
            "table-hl70203-to-v2-0203",
            "MR",
            "http://terminology.hl7.org/CodeSystem/v2-0203",
            "MR",
            "Medical record number",
        )
}

/// The Bundle as indented JSON, each number in the text it was written in.
pub(crate) fn pretty(bundle: &Value) -> String {
    serde_json::to_string_pretty(bundle).expect("the Bundle serializes")
}

/// The entries of the Bundle.
pub(crate) fn entries(bundle: &Value) -> &[Value] {
    bundle
        .get("entry")
        .and_then(Value::as_array)
        .unwrap_or_default()
}

/// The resources of the Bundle with `resource_type`.
pub(crate) fn resources<'b>(bundle: &'b Value, resource_type: &str) -> Vec<&'b Value> {
    entries(bundle)
        .iter()
        .filter_map(|entry| entry.get("resource"))
        .filter(|resource| {
            resource.get("resourceType").and_then(Value::as_str) == Some(resource_type)
        })
        .collect()
}

/// Maps a fixture with the stubbed tables and checks that the run counts
/// every translation it asked: one per target code system of the table map,
/// until one answers.
async fn mapped(bytes: &[u8]) -> (Mapped, Vec<(String, String)>) {
    let parsed = support::parsed(bytes);
    let corpus = support::corpus();
    let tables = tables();
    let (_server, client) = tables.serve().await;
    let mapped = map(&parsed, &corpus, Some(&client))
        .await
        .expect("the message maps");
    let asked = tables.asked();
    assert_eq!(
        mapped.translations(),
        asked.len(),
        "every translate is counted"
    );
    (mapped, asked)
}

/// A reviewable summary of the outcomes: kind, where, and the row.
pub(crate) fn summary(mapped: &Mapped) -> Vec<String> {
    mapped.outcomes().iter().map(line).collect()
}

/// One outcome as a line.
fn line(outcome: &Outcome) -> String {
    let kind = outcome.kind();
    match outcome {
        Outcome::Parse(unplaced) => format!("{kind}: {unplaced:?}"),
        Outcome::UnmappedSegment { at } | Outcome::UnmappedField { at } => format!("{kind}: {at}"),
        Outcome::UnmappedComponent { at, map, component } => {
            format!("{kind}: {at} {map} component {component}")
        }
        Outcome::NoSegmentMap {
            at,
            resource,
            candidates,
        } => format!("{kind}: {at} {resource} {candidates:?}"),
        Outcome::Undecodable {
            resource, error, ..
        } => format!("{kind}: {resource} {error}"),
        Outcome::MissingRequired {
            element, required, ..
        } => format!("{kind}: {element} lacks {required}"),
        other => row_line(kind, other),
    }
}

/// An outcome about one row, as a line.
fn row_line(kind: &str, outcome: &Outcome) -> String {
    let (at, row, detail) = match outcome {
        Outcome::NarrativeCondition { at, row }
        | Outcome::UnsupportedShape { at, row }
        | Outcome::NoTerminology { at, row }
        | Outcome::RepetitionDropped { at, row }
        | Outcome::UnplacedInstance { at, row }
        | Outcome::ComponentsDropped { at, row }
        | Outcome::Superseded { at, row }
        | Outcome::Untranslated { at, row, .. } => (at, row, String::new()),
        Outcome::UnsupportedCondition { at, row, text }
        | Outcome::DefectiveCondition { at, row, text }
        | Outcome::UnsupportedAssignment { at, row, text }
        | Outcome::DefectiveAssignment { at, row, text } => (at, row, text.clone()),
        Outcome::UnevaluableCondition { at, row, operand }
        | Outcome::UnevaluableAssignment { at, row, operand } => (at, row, operand.clone()),
        Outcome::UnresolvedTable {
            at,
            row,
            mapped_via,
        } => (at, row, mapped_via.clone()),
        Outcome::UnsupportedTarget { at, row, error } => (at, row, error.to_string()),
        Outcome::Unconvertible { at, row, error } => (at, row, error.to_string()),
        Outcome::UnknownElement { at, row, error } => (at, row, error.to_string()),
        Outcome::Unwritable { at, row, error } => (at, row, error.to_string()),
        Outcome::InvalidValue {
            at,
            row,
            element,
            fhir_type,
            error,
        } => (at, row, format!("{element} is no {fhir_type}: {error}")),
        Outcome::FacilityEndpoint { at, row, element } => (at, row, element.clone()),
        Outcome::FinestSibling { at, row, label }
        | Outcome::SiblingUnresolved { at, row, label } => (at, row, label.to_string()),
        Outcome::SiblingAmbiguous { at, row, labels } => (at, row, format!("{labels:?}")),
        Outcome::NoDatatypeMap {
            at,
            row,
            source_type,
            target_type,
        } => (at, row, format!("{source_type} to {target_type}")),
        Outcome::DatatypeConflict {
            at,
            row,
            element,
            maps,
        } => (at, row, format!("{element} from {}", maps.join(" "))),
        Outcome::DatatypeAmbiguous {
            at,
            row,
            candidates,
        } => (at, row, candidates.join(" ")),
        other => return format!("{kind}: {other:?}"),
    };
    format!(
        "{kind}: {at} {} {} -> {} [{detail}]",
        row.map, row.source, row.target
    )
}

#[test]
fn the_package_loads_every_concept_map_of_the_four_kinds() {
    let corpus = support::corpus();
    let count = |kind| corpus.maps().filter(|map| map.kind == kind).count();
    assert_eq!(count(Kind::Message), 13);
    assert_eq!(count(Kind::Segment), 75);
    assert_eq!(count(Kind::Datatype), 105);
    assert_eq!(count(Kind::Table), 70);
    assert_eq!(corpus.maps().count(), 263);
}

#[test]
fn the_corpus_conditions_parse_or_are_refused_as_counted_forms() {
    let corpus = support::corpus();
    let mut computable = 0usize;
    let mut refused = 0usize;
    let mut narrative = 0usize;
    let mut defective = BTreeMap::new();
    let mut unsupported = BTreeMap::new();
    for row in corpus.maps().flat_map(|map| map.rows.iter()) {
        match &row.condition {
            Condition::Computable(_) => computable += 1,
            Condition::Unsupported { text, .. } => {
                refused += 1;
                *unsupported.entry(text.clone()).or_insert(0usize) += 1;
            }
            Condition::Defective { text } => {
                refused += 1;
                *defective.entry(text.clone()).or_insert(0usize) += 1;
            }
            Condition::Narrative => narrative += 1,
            Condition::Always => {}
        }
    }
    let list = |forms: &BTreeMap<String, usize>| {
        forms
            .iter()
            .flat_map(|(text, count)| [format!("  {count} "), text.clone(), String::from("\n")])
            .collect::<String>()
    };
    insta::assert_snapshot!(
        "corpus_conditions",
        format!(
            "computable: {computable}\nrefused: {refused}\nnarrative: {narrative}\n\
             defective (no operand):\n{}unsupported:\n{}",
            list(&defective),
            list(&unsupported)
        )
    );
}

#[tokio::test]
async fn pid_maps_to_a_patient_that_decodes_as_r4() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    let patients = resources(mapped.bundle(), "Patient");
    assert_eq!(patients.len(), 1, "one PID, one Patient");
    let Value::Object(object) = patients[0] else {
        panic!("a Patient object");
    };
    let patient = fhir_types::r4::patient::Patient::from_json(object, &mut Path::root("Patient"))
        .expect("the Patient decodes as R4");
    assert_eq!(patient.name.len(), 1, "PID-5 is one name");
    assert_eq!(
        patient.name[0]
            .family
            .as_ref()
            .and_then(|family| family.value.as_deref()),
        Some("M\u{FC}ller"),
        "the Latin-1 family name arrives as text"
    );
    assert_eq!(
        patient
            .gender
            .as_ref()
            .and_then(|gender| gender.value.as_deref()),
        Some("male"),
        "PID-8 through table-hl70001-to-administrative-gender"
    );
    assert_eq!(
        patient
            .birth_date
            .as_ref()
            .and_then(|date| date.value.as_deref()),
        Some("1980-01-01")
    );
}

#[tokio::test]
async fn each_obx_maps_to_an_observation_that_decodes_as_r4() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    let observations = resources(mapped.bundle(), "Observation");
    assert_eq!(observations.len(), 2, "two OBX, two Observations");
    for (value, expected) in observations.iter().zip(["5.4", "140"]) {
        let Value::Object(object) = value else {
            panic!("an Observation object");
        };
        let observation = fhir_types::r4::observation::Observation::from_json(
            object,
            &mut Path::root("Observation"),
        )
        .expect("the Observation decodes as R4");
        assert_eq!(
            observation.status.value.as_deref(),
            Some("final"),
            "OBX-11 through table-hl70085-to-observation-status"
        );
        let written = value
            .get("valueQuantity")
            .and_then(|quantity| quantity.get("value"));
        assert!(
            matches!(written, Some(Value::Number(number)) if number.as_str() == expected),
            "OBX-5 as the quantity value: {written:?}"
        );
    }
}

#[tokio::test]
async fn the_bundle_is_a_message_with_the_message_header_first() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    let bundle = mapped.bundle();
    assert_eq!(bundle.get("type").and_then(Value::as_str), Some("message"));
    let first = entries(bundle)
        .first()
        .and_then(|entry| entry.get("resource"))
        .and_then(|resource| resource.get("resourceType"))
        .and_then(Value::as_str);
    assert_eq!(first, Some("MessageHeader"));
    for entry in entries(bundle) {
        assert!(
            entry
                .get("fullUrl")
                .and_then(Value::as_str)
                .is_some_and(|url| url.starts_with("urn:uuid:")),
            "every entry has a urn:uuid full url"
        );
    }
}

#[tokio::test]
async fn the_oru_r01_maps_to_the_reviewed_bundle() {
    let (mapped, asked) = mapped(&fixtures::oru_r01()).await;
    insta::assert_snapshot!("oru_r01_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("oru_r01_outcomes", summary(&mapped));
    insta::assert_debug_snapshot!("oru_r01_counts", mapped.counts());
    insta::assert_debug_snapshot!("oru_r01_translations", asked);
}

#[tokio::test]
async fn the_adt_a01_maps_to_the_reviewed_bundle() {
    let (mapped, asked) = mapped(&fixtures::adt_a01()).await;
    insta::assert_snapshot!("adt_a01_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("adt_a01_outcomes", summary(&mapped));
    insta::assert_debug_snapshot!("adt_a01_translations", asked);
}

#[tokio::test]
async fn the_mdm_t02_maps_to_the_reviewed_bundle() {
    let (mapped, asked) = mapped(&fixtures::mdm_t02()).await;
    insta::assert_snapshot!("mdm_t02_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("mdm_t02_outcomes", summary(&mapped));
    insta::assert_debug_snapshot!("mdm_t02_translations", asked);
}

#[tokio::test]
async fn a_z_segment_is_a_counted_outcome_of_the_mapping() {
    let (mapped, _) = mapped(&fixtures::oru_r01()).await;
    assert_eq!(mapped.counts().get("unknown-segment"), Some(&1));
}

#[tokio::test]
async fn a_table_map_with_no_terminology_server_is_a_counted_outcome() {
    let parsed = support::parsed(&fixtures::oru_r01());
    let corpus = support::corpus();
    let mapped = map(&parsed, &corpus, None).await.expect("the message maps");
    assert_eq!(mapped.translations(), 0);
    assert!(
        mapped
            .counts()
            .get("no-terminology")
            .is_some_and(|count| *count > 0)
    );
    let patients = resources(mapped.bundle(), "Patient");
    assert!(
        patients[0].get("gender").is_none(),
        "no code passes through untranslated"
    );
}

#[tokio::test]
async fn a_refused_translation_fails_the_run_with_the_upstream_status() {
    let parsed = support::parsed(&fixtures::oru_r01());
    let corpus = support::corpus();
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let base = format!("{}/r4", server.uri()).parse().expect("a base url");
    let config =
        ferrobridge_term::config::Config::new(base, ferrobridge_term::config::WireVersion::R4)
            .with_retry(ferrobridge_term::config::RetryPolicy {
                max_attempts: 1,
                initial_backoff: std::time::Duration::from_millis(1),
                max_backoff: std::time::Duration::from_millis(1),
            });
    let client = ferrobridge_term::client::Client::new(config).expect("a client");
    let refused = map(&parsed, &corpus, Some(&client)).await;
    assert!(
        matches!(
            refused,
            Err(ferrobridge_hl7v2::map::MapError::Terminology { .. })
        ),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_supplement_map_replaces_the_package_map_with_its_url() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let package = support::package().join("ConceptMap-segment-pv1-to-encounter.json");
    let text = std::fs::read_to_string(&package).expect("the package map reads");
    let mut map_json: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let first = map_json["group"][0]["element"][0].clone();
    let targets = first["target"].as_array().map_or(0, Vec::len);
    map_json["group"][0]["element"] = serde_json::json!([first]);
    std::fs::write(
        directory
            .path()
            .join("ConceptMap-segment-pv1-to-encounter.json"),
        serde_json::to_vec(&map_json).expect("JSON"),
    )
    .expect("the supplement is written");
    let corpus = support::corpus()
        .supplement(directory.path())
        .expect("the supplement loads");
    assert!(
        corpus
            .get("segment-pv1-to-encounter")
            .is_some_and(|map| map.rows.len() == targets),
        "the supplement's map took the package map's place"
    );
    assert_eq!(corpus.maps().count(), 263);
}

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

// NOTE: `mapping_guidelines.md` §General Format/Approach: a condition decides whether the v2
// element is mapped, so a row on an empty PID-13 writes no `telecom[1].use` without a value.
#[tokio::test]
async fn a_row_on_an_empty_field_whose_condition_names_another_writes_nothing() {
    let (mapped, _) = mapped(&fixtures::oru_r01_named_applications()).await;
    let patient = resources(mapped.bundle(), "Patient");
    assert_eq!(patient.len(), 1);
    assert!(
        patient
            .iter()
            .all(|patient| patient.get("telecom").is_none()),
        "{patient:?}"
    );
}
