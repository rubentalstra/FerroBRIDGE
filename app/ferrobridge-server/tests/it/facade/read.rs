// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The read: the id a composition answers under, the subject, and the
//! absent and deleted cases.

use axum::body::Body;
use ferrobridge_server::facade::identity::store::Store;
use http::Request;
use http::StatusCode;
use std::error::Error as StdError;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::CONTAINER;
use super::EHR_ID;
use super::VERSION_ONE;
use super::VERSION_TWO;
use super::call;
use super::create_one;
use super::first_issue;
use super::harness;
use super::stub::mount_read;

#[tokio::test]
async fn the_same_composition_read_twice_yields_the_same_id() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let first = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    let second = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, first.0, "{:?}", first.1);
    assert_eq!(first.1["id"], second.1["id"]);
    assert_eq!(Some(id.as_str()), first.1["id"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_read_names_the_subject_the_identity_map_binds_to_the_ehr()
-> Result<(), Box<dyn StdError>> {
    // R4 condition.html: Condition.subject is 1..1, and the fixture mapping
    // has no outbound row for it, so the facade writes it from the binding.
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(
        serde_json::json!({
            "type": "Patient",
            "identifier": {
                "system": "http://example.org/fhir/sid/ferrobridge-subject",
                "value": "synthetic-subject-0001"
            }
        }),
        body["subject"],
        "{body}"
    );
    let vread = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}/_history/1")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, vread.0, "{}", vread.1);
    assert_eq!(body["subject"], vread.1["subject"]);
    Ok(())
}

#[tokio::test]
async fn a_read_whose_ehr_has_no_recorded_person_is_refused_naming_the_element()
-> Result<(), Box<dyn StdError>> {
    // A read never answers an instance R4 does not admit: without a person
    // bound to the EHR, Condition.subject (1..1) stays absent.
    let harness = harness().await;
    let id = ferrobridge_server::facade::identity::FhirResourceId::new("unboundsubject")?;
    harness.store.record_binding(
        "Condition",
        &id,
        &ferrobridge_server::facade::identity::record::CompositionBinding {
            ehr_id: String::from(EHR_ID),
            versioned_object_uid: String::from(CONTAINER),
            template_id: String::from("ferrobridge.diagnose.v1"),
            resource_type: String::from("Condition"),
            entry_path: String::from("/content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]"),
            split: 0,
            context: String::from("ferrobridge_facade.context"),
        },
    )?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/Condition/unboundsubject").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let issue = first_issue(&body);
    assert_eq!(Some("exception"), issue["code"].as_str());
    assert_eq!(
        Some("Condition.subject"),
        issue["location"][0].as_str(),
        "{body}"
    );
    assert!(
        !body.to_string().contains("Synthetic problem one"),
        "the refusal carries clinical content: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_new_version_keeps_the_id_and_moves_the_version_id() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_TWO, &harness.composition()).await;
    let (status, body) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some(id.as_str()), body["id"].as_str());
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_read_of_an_id_the_map_does_not_know_is_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(Some("not-found"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_composition_the_cdr_reports_deleted_is_four_hundred_and_ten()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::GONE, status);
    assert_eq!(Some("deleted"), first_issue(&body)["code"].as_str());
    Ok(())
}
