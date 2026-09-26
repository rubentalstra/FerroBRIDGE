// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The counted outcomes and the failures of a run.

use ferrobridge_hl7v2::map::map;

use crate::fixtures;
use crate::support::{self};

use crate::map::{mapped, resources};

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
