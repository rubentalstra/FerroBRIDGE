// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The supplements the crate ships, each on the row it changes: the message
//! run through the guide alone and through the guide with the supplements,
//! as the face runs it.

use ferrobridge_hl7v2::map::corpus::{Condition, Corpus};
use ferrobridge_hl7v2::map::{Mapped, Outcome, map};
use fhir_types::codec::Value;

use crate::fixtures;
use crate::map::{entries, pretty, resources, summary, tables};
use crate::support;

/// Maps `bytes` through `corpus` with no terminology server.
async fn run(bytes: &[u8], corpus: &Corpus) -> Mapped {
    map(&support::parsed(bytes), corpus, None)
        .await
        .expect("the message maps")
}

/// Maps a fixture through the shipped supplements with the stubbed tables.
async fn reviewed(bytes: &[u8]) -> Mapped {
    translated(bytes, &support::shipped()).await
}

/// Maps `bytes` through `corpus` with the stubbed tables and the answers of
/// the report status, order status and specimen type tables the fixtures
/// here reach, read from those table maps of the guide.
async fn translated(bytes: &[u8], corpus: &Corpus) -> Mapped {
    let tables = tables()
        .answer(
            "table-hl70123-queries-to-diagnostic-report-status",
            "F",
            "http://hl7.org/fhir/diagnostic-report-status",
            "final",
            "Final",
        )
        .answer(
            "table-hl70119-to-request-status",
            "OK",
            "http://hl7.org/fhir/request-status",
            "active",
            "Active",
        )
        .answer(
            "table-hl70487-to-v2-0487",
            "BLD",
            "http://terminology.hl7.org/CodeSystem/v2-0487",
            "BLD",
            "Whole blood",
        );
    let (_server, client) = tables.serve().await;
    map(&support::parsed(bytes), corpus, Some(&client))
        .await
        .expect("the message maps")
}

/// The ORU^R01 of [`fixtures::oru_r01`] with its report final (OBR-25 `F`),
/// so the `DiagnosticReport` carries the status R4 requires.
fn final_result() -> Vec<u8> {
    let mut obr = vec!["OBR", "1", "PLC-1", "FIL-1", "2345-7^Glucose^LN"];
    obr.extend([""; 20]);
    obr.push("F");
    let obr = obr.join("|");
    fixtures::message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00043|P|2.5.1",
        b"PID|1||PAT-0043^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        obr.as_bytes(),
        b"OBX|1|NM|2345-7^Glucose^LN||5.4|mmol/L^mmol/L^UCUM|3.9-5.8|N|||F",
    ])
}

/// The one resource of `resource_type` in the Bundle.
fn only<'b>(bundle: &'b Value, resource_type: &str) -> &'b Value {
    let found = resources(bundle, resource_type);
    assert_eq!(found.len(), 1, "the resources of {resource_type}");
    found.first().expect("one resource")
}

/// The ids of the supplement maps a run used, each with whether it replaced
/// a guide map.
fn supplemented(mapped: &Mapped) -> Vec<(&str, bool)> {
    mapped
        .outcomes()
        .iter()
        .filter_map(|outcome| match outcome {
            Outcome::Supplemented { map, overrides, .. } => Some((map.as_str(), *overrides)),
            _ => None,
        })
        .collect()
}

/// The first Coding of the `CodeableConcept` at `key` of `resource`.
fn first_coding<'v>(resource: &'v Value, key: &str) -> &'v Value {
    resource
        .get(key)
        .and_then(|concept| concept.get("coding"))
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .expect("a coding")
}

fn text(value: &str) -> Value {
    Value::String(String::from(value))
}

// NOTE: `datatype-cwe-to-codeableconcept` gates CWE.1 on a Narrative-Condition; the
// supplement runs the row, so OBR-4 gives the code beside the guide's display and system.
#[tokio::test]
async fn cwe_one_is_the_code_of_the_coding_the_codeableconcept_map_writes() {
    let guide = translated(&final_result(), &support::corpus()).await;
    let report = only(guide.bundle(), "DiagnosticReport");
    assert_eq!(first_coding(report, "code").get("code"), None);
    let shipped = translated(&final_result(), &support::shipped()).await;
    for resource_type in ["DiagnosticReport", "Observation"] {
        let coding = first_coding(only(shipped.bundle(), resource_type), "code");
        assert_eq!(coding.get("code"), Some(&text("2345-7")), "{resource_type}");
        assert_eq!(
            coding.get("display"),
            Some(&text("Glucose")),
            "{resource_type}"
        );
        assert_eq!(coding.get("system"), Some(&text("LN")), "{resource_type}");
    }
    assert!(
        shipped.outcomes().iter().all(|outcome| !matches!(
            outcome,
            Outcome::NarrativeCondition { row, .. }
                if row.map == "datatype-cwe-to-codeableconcept" && row.source == "CWE.1"
        )),
        "{:?}",
        shipped.outcomes()
    );
    assert!(supplemented(&shipped).contains(&("datatype-cwe-to-codeableconcept", true)));
}

// NOTE: `datatype-cf-to-codeableconcept` names its sources CWE.1 to CWE.13; the supplement
// names the CF components and runs CF.1 with no condition.
#[test]
fn the_cf_map_reads_the_cf_components_with_an_ungated_code() {
    let guide = support::corpus();
    let package = guide
        .get("datatype-cf-to-codeableconcept")
        .expect("the guide's map");
    assert!(
        package
            .rows
            .iter()
            .all(|row| row.source.starts_with("CWE."))
    );
    let shipped = support::shipped();
    let map = shipped
        .get("datatype-cf-to-codeableconcept")
        .expect("the supplement");
    assert_eq!(map.rows.len(), package.rows.len());
    assert!(map.rows.iter().all(|row| row.source.starts_with("CF.")));
    let code = map
        .rows
        .iter()
        .find(|row| row.target_code == "coding[1].code")
        .expect("the code row");
    assert_eq!(
        (code.source.as_str(), &code.condition),
        ("CF.1", &Condition::Always)
    );
}

// NOTE: `datatype-cwe-to-quantity` names its targets `Quantity.code`, `.unit` and `.system`;
// the supplement names the Quantity elements, so OBX-6 gives the units.
#[tokio::test]
async fn obx_six_gives_the_units_of_the_quantity() {
    let guide = translated(&fixtures::oru_r01(), &support::corpus()).await;
    let shipped = reviewed(&fixtures::oru_r01()).await;
    let quantity = |mapped: &Mapped| -> Vec<Value> {
        resources(mapped.bundle(), "Observation")
            .into_iter()
            .filter_map(|observation| observation.get("valueQuantity").cloned())
            .collect()
    };
    assert!(
        quantity(&guide)
            .iter()
            .all(|value| value.get("unit").is_none()),
        "{:?}",
        quantity(&guide)
    );
    let found = quantity(&shipped);
    assert_eq!(found.len(), 2, "{}", pretty(shipped.bundle()));
    for value in found {
        assert_eq!(value.get("code"), Some(&text("mmol/L")), "{value:?}");
        assert_eq!(value.get("unit"), Some(&text("mmol/L")), "{value:?}");
        assert_eq!(value.get("system"), Some(&text("UCUM")), "{value:?}");
    }
    assert!(
        shipped.outcomes().iter().all(|outcome| !matches!(
            outcome,
            Outcome::UnknownElement { row, .. } if row.map == "datatype-cwe-to-quantity"
        )),
        "{:?}",
        shipped.outcomes()
    );
}

/// A result from an application in MSH-3 whose network address is MSH-24.
fn application_and_network() -> Vec<u8> {
    fixtures::message(&[
        b"MSH|^~\\&|LAB^1.2.3.4.5^ISO|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00045|P|2.5.1||||||||||||NORTHNET^1.2.3.9^ISO",
        b"PID|1||PAT-0045^^^NORTHLAB^MR||Doe^Sam^^^^^L||19800101|M",
        b"ORC|RE|PLC-1|FIL-1",
        b"OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN",
    ])
}

// NOTE: `segment-msh-to-messageheader` gates the MSH-3 endpoint row on IF MSH-24 NOT VALUED;
// with both valued, the application names the source and the network address gives the endpoint.
#[tokio::test]
async fn a_network_address_gives_the_endpoint_beside_the_application_name() {
    let shipped = run(&application_and_network(), &support::shipped()).await;
    let header = only(shipped.bundle(), "MessageHeader");
    let source = header.get("source").expect("a source");
    assert_eq!(source.get("endpoint"), Some(&text("urn:oid:1.2.3.9")));
    assert_eq!(source.get("name"), Some(&text("LAB")));
    assert_eq!(source.get("software"), None, "{source:?}");
    assert!(
        shipped
            .outcomes()
            .iter()
            .all(|outcome| !matches!(outcome, Outcome::DatatypeConflict { .. })),
        "{:?}",
        shipped.outcomes()
    );
    let guide = run(&application_and_network(), &support::corpus()).await;
    let source = only(guide.bundle(), "MessageHeader")
        .get("source")
        .expect("a source");
    assert_eq!(source.get("endpoint"), Some(&text("urn:oid:1.2.3.4.5")));
    assert_eq!(source.get("software"), Some(&text("1.2.3.4.5")));
}

// NOTE: `datatype-hd-name-to-messageheader-destination` writes HD.2 into `name` where the
// endpoint map writes HD.1; the supplement writes HD.1, so the two maps agree.
#[tokio::test]
async fn a_destination_network_address_gives_one_name() {
    let message =
        fixtures::oru_r01_network_addresses("NORTHNET^1.2.3.9^ISO", "SOUTHNET^1.2.3.10^ISO");
    let guide = run(&message, &support::corpus()).await;
    assert!(
        guide.outcomes().iter().any(|outcome| matches!(
            outcome,
            Outcome::DatatypeConflict { element, .. } if element == "name"
        )),
        "{:?}",
        guide.outcomes()
    );
    let shipped = run(&message, &support::shipped()).await;
    let destination = only(shipped.bundle(), "MessageHeader")
        .get("destination")
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .expect("a destination");
    assert_eq!(destination.get("name"), Some(&text("SOUTHNET")));
    assert_eq!(destination.get("endpoint"), Some(&text("urn:oid:1.2.3.10")));
    assert!(
        shipped
            .outcomes()
            .iter()
            .all(|outcome| !matches!(outcome, Outcome::DatatypeConflict { .. })),
        "{:?}",
        shipped.outcomes()
    );
}

// NOTE: `segment-orc-to-diagnosticreport` maps ORC-2 into basedOn(ServiceRequest); the
// supplement writes the placer number into basedOn.identifier (R4 Reference.identifier).
#[tokio::test]
async fn a_placer_order_number_is_the_identifier_the_report_is_based_on() {
    let guide = translated(&final_result(), &support::corpus()).await;
    assert!(
        guide.outcomes().iter().any(|outcome| matches!(
            outcome,
            Outcome::NoDatatypeMap { row, .. } if row.source == "ORC-2"
        )),
        "{:?}",
        guide.outcomes()
    );
    let shipped = translated(&final_result(), &support::shipped()).await;
    let based_on = only(shipped.bundle(), "DiagnosticReport")
        .get("basedOn")
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .expect("a basedOn");
    assert_eq!(
        based_on.get("identifier").and_then(|id| id.get("value")),
        Some(&text("PLC-1")),
        "{based_on:?}"
    );
    assert_eq!(based_on.get("reference"), None);
    assert!(resources(shipped.bundle(), "ServiceRequest").is_empty());
}

/// A message of `event` whose structure the guide maps with no message map
/// of its own, from MSH, EVN, PID and PV1.
fn admission(event: &str, control: &str) -> Vec<u8> {
    let header = format!(
        "MSH|^~\\&|ADMIT|NORTHHOSP|EHR|SOUTHCLINIC|20260926090000+0200||{event}|{control}|P|2.5.1"
    );
    fixtures::message(&[
        header.as_bytes(),
        b"EVN||20260926085900+0200",
        b"PID|1||PAT-0047^^^NORTHHOSP^MR||Doe^Sam^^^^^L||19800101|M",
        b"PV1|1|I|WARD1^101^A",
    ])
}

// NOTE: the message maps the supplements add (ADT_A03, BAR_P01) and fix (ADT_A05 and ADT_A09
// name ADT_A01.PD1) map their structures with the guide's segment maps.
#[tokio::test]
async fn the_supplemented_message_maps_carry_their_structures() {
    for (event, map_id, overrides) in [
        ("ADT^A03^ADT_A03", "message-adt-a03-to-bundle", false),
        ("ADT^A28^ADT_A05", "message-adt-a05-to-bundle", true),
        ("ADT^A09^ADT_A09", "message-adt-a09-to-bundle", true),
        ("BAR^P01^BAR_P01", "message-bar-p01-to-bundle", false),
    ] {
        let message = admission(event, "MSG00047");
        let refused = map(&support::parsed(&message), &support::corpus(), None).await;
        assert!(
            matches!(
                refused,
                Err(ferrobridge_hl7v2::map::MapError::NoMessageMap { .. })
            ),
            "{event}: {refused:?}"
        );
        let shipped = run(&message, &support::shipped()).await;
        let patient = only(shipped.bundle(), "Patient");
        assert!(patient.get("identifier").is_some(), "{event}: {patient:?}");
        assert!(
            supplemented(&shipped).contains(&(map_id, overrides)),
            "{event}: {:?}",
            supplemented(&shipped)
        );
    }
    let billed = run(
        &admission("BAR^P01^BAR_P01", "MSG00048"),
        &support::shipped(),
    )
    .await;
    assert_eq!(resources(billed.bundle(), "Account").len(), 1);
}

// NOTE: no specification governs this: our own design; each supplement map is counted
// once per message, where it first ran, and the guide alone counts none.
#[tokio::test]
async fn each_supplement_a_run_uses_is_counted_once() {
    let guide = run(&fixtures::oru_r01(), &support::corpus()).await;
    assert!(supplemented(&guide).is_empty());
    let shipped = run(&fixtures::oru_r01(), &support::shipped()).await;
    let used = supplemented(&shipped);
    let mut unique = used.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(used.len(), unique.len(), "{used:?}");
    assert!(
        used.contains(&("segment-msh-to-messageheader", true)),
        "{used:?}"
    );
}

#[tokio::test]
async fn the_oul_r22_maps_to_the_reviewed_bundle() {
    let mapped = reviewed(&fixtures::oul_r22()).await;
    insta::assert_snapshot!("oul_r22_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("oul_r22_outcomes", summary(&mapped));
    assert!(supplemented(&mapped).contains(&("message-oul-r22-to-bundle", false)));
}

#[tokio::test]
async fn the_orl_o22_maps_to_the_reviewed_bundle() {
    let mapped = reviewed(&fixtures::orl_o22()).await;
    insta::assert_snapshot!("orl_o22_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!("orl_o22_outcomes", summary(&mapped));
    assert!(supplemented(&mapped).contains(&("message-orl-o22-to-bundle", false)));
}

/// The guide's own sample pair: the MDM^T02 message of `HL7/v2-to-fhir`
/// `samples/messages/` and the Bundle its `samples/fhir-bundles/` holds for
/// it, vendored by `scripts/vendor/hl7v2-samples.sh`.
const SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/vendor/v2-to-fhir/samples/messages/Message.hl7_MDM_T02.txt"
);

/// The sample's Bundle; the `.json` file of the pair holds XML at the pin.
const SAMPLE_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/vendor/v2-to-fhir/samples/fhir-bundles/FHIR_bundle.hl7_MDM_T02.xml"
);

/// The resource types of a Bundle's entries, sorted with their counts.
fn inventory(bundle: &Value) -> Vec<(String, usize)> {
    let mut counts = std::collections::BTreeMap::new();
    for kind in entries(bundle)
        .iter()
        .filter_map(|entry| entry.get("resource"))
        .filter_map(|resource| resource.get("resourceType"))
        .filter_map(Value::as_str)
    {
        let count = counts.entry(kind.to_owned()).or_insert(0usize);
        *count = count.saturating_add(1);
    }
    counts.into_iter().collect()
}

// NOTE: HL7 v2-to-FHIR `samples/`: the one message the repository pairs with its Bundle; the
// Bundle is another converter's, so the resource inventories are a reviewed comparison.
#[tokio::test]
async fn the_guides_mdm_t02_sample_maps_to_the_reviewed_bundle() {
    let bytes = std::fs::read(SAMPLE).expect("the vendored sample reads");
    let text = String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\r")
        .replace('\n', "\r");
    let mapped = {
        let tables = tables();
        let (_server, client) = tables.serve().await;
        map(
            &support::parsed(text.trim_end_matches('\r').as_bytes()),
            &support::shipped(),
            Some(&client),
        )
        .await
        .expect("the sample maps")
    };
    let expected = std::fs::read_to_string(SAMPLE_BUNDLE).expect("the vendored Bundle reads");
    let expected = fhir_types::xml::from_xml(&fhir_types::r4::schema::SCHEMAS, &expected)
        .map(Value::Object)
        .expect("the vendored Bundle decodes");
    insta::assert_snapshot!("mdm_t02_sample_bundle", pretty(mapped.bundle()));
    insta::assert_debug_snapshot!(
        "mdm_t02_sample_inventories",
        (inventory(mapped.bundle()), inventory(&expected))
    );
}
