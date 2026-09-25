// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ingest service, driven with no HTTP request at all.
//!
//! Every case hands the service a Bundle value and reads the typed result,
//! over the same `wiremock` CDR the facade suite stubs, so the two rules for
//! an entry no program maps are asserted at the seam a non-HTTP face calls.

use crate::facade::EHR_ID;
use crate::facade::client;
use crate::facade::ehr_body;
use crate::facade::handle;
use crate::facade::settings;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::ingest::EntryOutcome;
use ferrobridge_server::facade::ingest::Provenance;
use ferrobridge_server::facade::ingest::SkipReason;
use ferrobridge_server::facade::ingest::UnmappedEntries;
use ferrobridge_server::facade::outcome::IssueType;
use ferrobridge_server::facade::programs;
use fhirconnect::engine::origin::MessageControlId;
use fhirconnect::engine::origin::MessageType;
use fhirconnect::engine::origin::SourceItem;
use http::StatusCode;
use std::error::Error as StdError;
use std::sync::Arc;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

/// The mapping set this suite loads: one Observation context.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/ingest_fixtures");

/// The profile the Observation context claims.
const PROFILE: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-ingest-observation";

/// The contribution the stub CDR answers a commit with.
const CONTRIBUTION: &str = "7b0a4c2e-0000-4000-8000-00000000000c";

/// The control id of the synthetic message the Bundle stands for.
const CONTROL_ID: &str = "MSG-0001";

/// Returns the provenance of the synthetic `ORU^R01` message.
fn message() -> Provenance {
    Provenance::Item(SourceItem::message(
        MessageControlId::new(CONTROL_ID).expect("a non-empty control id"),
        MessageType::new("ORU^R01").expect("a non-empty message type"),
    ))
}

/// Returns a facade over the Observation mapping set and a stub CDR that
/// serves its template, resolves the EHR and takes a contribution.
async fn facade() -> Result<(MockServer, Facade), Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(
            "/definition/template/adl1.4/ferrobridge.diagnose.v1",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(ferrobridge_testkit::fixtures::DIAGNOSE_OPT),
        )
        .mount(&cdr)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ehr_body()))
        .mount(&cdr)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(
            ResponseTemplate::new(201).insert_header("ETag", format!("W/\"{CONTRIBUTION}\"")),
        )
        .mount(&cdr)
        .await;
    let set = programs::read_set(std::path::Path::new(FIXTURES))?;
    let templates = programs::fetch_templates(&set, &client(&cdr)).await?;
    let compiled = programs::compile_set(&set, &templates)?;
    let store = Arc::new(MemoryStore::new());
    let facade = Facade::new(compiled, handle(&store), client(&cdr), settings());
    Ok((cdr, facade))
}

/// Returns the synthetic Observation, claiming [`PROFILE`].
fn observation() -> serde_json::Value {
    let mut resource: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_OBSERVATION)
            .expect("the synthetic Observation reads");
    resource["meta"] = serde_json::json!({ "profile": [PROFILE] });
    resource
}

/// Returns a Bundle of a Patient, a Practitioner and `observation`, in that
/// order, the shape a message face builds beside the resources it commits.
fn bundle(observation: &serde_json::Value) -> fhir_types::codec::Value {
    let document = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-patient", "resource": {
                "resourceType": "Patient",
                "id": "ferrobridge-synthetic-patient-1",
                "identifier": [{
                    "system": "http://example.org/fhir/sid/ferrobridge-subject",
                    "value": "synthetic-subject-0001"
                }]
            } },
            { "fullUrl": "urn:uuid:0000-practitioner", "resource": {
                "resourceType": "Practitioner",
                "id": "ferrobridge-synthetic-practitioner-1",
                "name": [{ "family": "Synthetic" }]
            } },
            { "fullUrl": "urn:uuid:0000-observation", "resource": observation }
        ]
    });
    fhir_types::codec::Value::from_serde_json(document)
}

/// Returns how many contributions the stub CDR received.
async fn contributions(cdr: &MockServer) -> usize {
    cdr.received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| {
            request.method.as_str() == "POST"
                && request.url.path() == format!("/ehr/{EHR_ID}/contribution")
        })
        .count()
}

#[tokio::test]
async fn skip_and_count_commits_the_mapped_entry_and_types_every_skip()
-> Result<(), Box<dyn StdError>> {
    let (cdr, facade) = facade().await?;
    let provenance = message();
    let ingested = facade
        .ingest(facade.client().clone())
        .ingest_bundle(
            &bundle(&observation()),
            UnmappedEntries::SkipAndCount,
            &provenance,
        )
        .await
        .map_err(|refused| format!("the Bundle was refused: {refused:?}"))?;

    let skipped: Vec<(&str, &str, &SkipReason)> = ingested
        .skipped()
        .map(|skip| {
            (
                skip.full_url.as_str(),
                skip.resource_type.as_str(),
                &skip.reason,
            )
        })
        .collect();
    assert_eq!(
        vec![
            (
                "urn:uuid:0000-patient",
                "Patient",
                &SkipReason::NoProgramForType
            ),
            (
                "urn:uuid:0000-practitioner",
                "Practitioner",
                &SkipReason::NoProgramForType
            ),
        ],
        skipped
    );
    let committed: Vec<(&str, &str)> = ingested
        .committed()
        .map(|entry| (entry.full_url.as_str(), entry.resource_type.as_str()))
        .collect();
    assert_eq!(
        vec![("urn:uuid:0000-observation", "Observation")],
        committed
    );
    assert_eq!(3, ingested.entries().len(), "one outcome per entry");
    assert!(matches!(
        ingested.entries().last(),
        Some(EntryOutcome::Committed(_))
    ));
    assert_eq!(CONTRIBUTION, ingested.contribution().as_str());
    assert_eq!(EHR_ID, ingested.ehr_id().as_str());
    assert_eq!(1, contributions(&cdr).await, "the one mapped entry commits");
    Ok(())
}

#[tokio::test]
async fn a_message_origin_names_the_control_id_in_the_feeder_audit() -> Result<(), Box<dyn StdError>>
{
    let (cdr, facade) = facade().await?;
    let provenance = message();
    facade
        .ingest(facade.client().clone())
        .ingest_bundle(
            &bundle(&observation()),
            UnmappedEntries::SkipAndCount,
            &provenance,
        )
        .await
        .map_err(|refused| format!("the Bundle was refused: {refused:?}"))?;
    let received = cdr.received_requests().await.unwrap_or_default();
    let sent: serde_json::Value = received
        .iter()
        .find(|request| request.url.path() == format!("/ehr/{EHR_ID}/contribution"))
        .map(|request| serde_json::from_slice(&request.body))
        .ok_or("the Bundle sent a contribution")??;
    let versions = sent["versions"]
        .as_array()
        .ok_or("the contribution carries versions")?;
    assert_eq!(1, versions.len(), "one version per committed entry: {sent}");
    let item = &versions[0]["data"]["feeder_audit"]["originating_system_item_ids"][0];
    assert_eq!(
        Some(CONTROL_ID),
        item["id"].as_str(),
        "the message control id travels as the originating item: {item}"
    );
    assert_eq!(Some("ORU^R01"), item["type"].as_str());
    Ok(())
}

#[tokio::test]
async fn refuse_names_every_unmapped_entry_and_commits_nothing() -> Result<(), Box<dyn StdError>> {
    let (cdr, facade) = facade().await?;
    let refused = facade
        .ingest(facade.client().clone())
        .ingest_bundle(
            &bundle(&observation()),
            UnmappedEntries::Refuse,
            &Provenance::EachResource,
        )
        .await
        .err()
        .ok_or("a Bundle with unmapped entries is refused under Refuse")?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, refused.status());
    let codes: Vec<IssueType> = refused
        .issues()
        .iter()
        .map(ferrobridge_server::facade::outcome::Issue::code)
        .collect();
    assert_eq!(
        vec![IssueType::NotSupported, IssueType::NotSupported],
        codes
    );
    let rendered: Vec<String> = refused
        .issues()
        .iter()
        .map(|issue| format!("{:?}", issue.build().location))
        .collect();
    assert!(
        rendered
            .first()
            .is_some_and(|text| text.contains("urn:uuid:0000-patient")),
        "{rendered:?}"
    );
    assert!(
        rendered
            .get(1)
            .is_some_and(|text| text.contains("urn:uuid:0000-practitioner")),
        "{rendered:?}"
    );
    assert_eq!(
        0,
        contributions(&cdr).await,
        "a refused Bundle commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn skip_and_count_types_a_profile_no_program_claims() -> Result<(), Box<dyn StdError>> {
    let (cdr, facade) = facade().await?;
    let mut unclaimed = observation();
    unclaimed["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    let document = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-observation", "resource": observation() },
            { "fullUrl": "urn:uuid:0000-unclaimed", "resource": unclaimed }
        ]
    });
    let ingested = facade
        .ingest(facade.client().clone())
        .ingest_bundle(
            &fhir_types::codec::Value::from_serde_json(document),
            UnmappedEntries::SkipAndCount,
            &Provenance::EachResource,
        )
        .await
        .map_err(|refused| format!("the Bundle was refused: {refused:?}"))?;
    let reasons: Vec<&SkipReason> = ingested.skipped().map(|skip| &skip.reason).collect();
    assert_eq!(
        vec![&SkipReason::NoProgramForProfiles {
            profiles: vec![String::from("http://example.org/other")]
        }],
        reasons
    );
    assert_eq!(1, ingested.committed().count());
    assert_eq!(1, contributions(&cdr).await);
    Ok(())
}

#[tokio::test]
async fn a_bundle_whose_every_entry_is_skipped_commits_nothing_and_is_refused()
-> Result<(), Box<dyn StdError>> {
    let (cdr, facade) = facade().await?;
    let document = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-practitioner", "resource": {
                "resourceType": "Practitioner",
                "name": [{ "family": "Synthetic" }]
            } }
        ]
    });
    let refused = facade
        .ingest(facade.client().clone())
        .ingest_bundle(
            &fhir_types::codec::Value::from_serde_json(document),
            UnmappedEntries::SkipAndCount,
            &Provenance::EachResource,
        )
        .await
        .err()
        .ok_or("a Bundle with nothing to commit is refused")?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, refused.status());
    assert_eq!(
        vec![IssueType::Required],
        refused
            .issues()
            .iter()
            .map(ferrobridge_server::facade::outcome::Issue::code)
            .collect::<Vec<_>>()
    );
    assert_eq!(0, contributions(&cdr).await);
    Ok(())
}
