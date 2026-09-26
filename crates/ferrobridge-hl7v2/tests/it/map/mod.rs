// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The v2-to-FHIR `ConceptMap` interpreter over the vendored
//! `hl7.fhir.uv.v2mappings` 1.0.0 package, with the table maps answered by a
//! stubbed terminology server.

mod bundle;
mod endpoint;
mod outcome;
mod package;

use ferrobridge_hl7v2::map::{Mapped, Outcome, map};
use fhir_types::codec::Value;

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
pub(super) async fn mapped(bytes: &[u8]) -> (Mapped, Vec<(String, String)>) {
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
pub(super) fn line(outcome: &Outcome) -> String {
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
