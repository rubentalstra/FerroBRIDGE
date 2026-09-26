// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The transaction Bundle: all or nothing, one contribution, redelivery and
//! overlap.

use axum::body::Body;
use ferrobridge_server::facade::identity::store::Store;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use std::time::Duration;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::BASE_URL;
use super::CONTAINER;
use super::EHR_ID;
use super::VERSION_ONE;
use super::call;
use super::condition;
use super::condition_at;
use super::condition_source_at;
use super::first_issue;
use super::harness;
use super::post_condition;
use super::post_transaction;
use super::stub::ECHOED;
use super::stub::Echo;
use super::stub::mount_echo;
use super::stub::mount_echo_after;
use super::stub::mount_ehr;
use super::transaction_of;
use super::writes;

#[tokio::test]
async fn a_transaction_with_one_failing_entry_commits_nothing_and_names_it()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let mut broken = condition();
    broken["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-good", "resource": condition(),
              "request": { "method": "POST", "url": "Condition" } },
            { "fullUrl": "urn:uuid:0000-bad", "resource": broken,
              "request": { "method": "POST", "url": "Condition" } }
        ]
    });
    let request = Request::post("/fhir")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(bundle.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    assert!(
        body.to_string().contains("urn:uuid:0000-bad"),
        "the refusal names the failing entry by its fullUrl: {body}"
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "a transaction that cannot be mapped in full commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_transaction_that_maps_in_full_commits_one_contribution() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Sent).await;
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-good", "resource": condition(),
              "request": { "method": "POST", "url": "Condition" } }
        ]
    });
    let request = Request::post("/fhir")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(bundle.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the whole Bundle commits as one contribution"
    );
    Ok(())
}

/// Mounts the echo CDR whose first commit produces [`VERSION_ONE`].
async fn mount_contribution(cdr: &MockServer) {
    mount_echo(cdr, ECHOED, Echo::Sent).await;
}

#[tokio::test]
async fn each_transaction_entry_answers_the_location_and_etag_a_create_answers()
-> Result<(), Box<dyn StdError>> {
    // A transaction answers a `transaction-response` Bundle whose entries each
    // carry the status, `location` and `ETag`
    // (<https://hl7.org/fhir/R4/http.html#transaction-response>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some("Bundle"), body["resourceType"].as_str(), "{body}");
    assert_eq!(Some("transaction-response"), body["type"].as_str());
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("201 Created"), response["status"].as_str(), "{body}");
    assert_eq!(Some("W/\"1\""), response["etag"].as_str());
    let location = response["location"]
        .as_str()
        .ok_or("the entry answers a location")?;
    let id = location
        .strip_prefix(&format!("{BASE_URL}/Condition/"))
        .and_then(|rest| rest.strip_suffix("/_history/1"))
        .ok_or(format!("{location} is no [base]/Condition/[id]/_history/1"))?;
    let internal = ferrobridge_server::facade::identity::FhirResourceId::new(id)?;
    let binding = harness
        .store
        .binding_of("Condition", &internal)?
        .ok_or("the transaction recorded the entry's identity binding")?;
    assert_eq!(CONTAINER, binding.versioned_object_uid);
    assert_eq!(EHR_ID, binding.ehr_id);
    Ok(())
}

#[tokio::test]
async fn a_transaction_sent_twice_commits_once_and_answers_the_same_ids()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition())]);
    let (first_status, first) = post_transaction(&harness, &bundle).await?;
    let (second_status, second) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    assert_eq!(StatusCode::OK, second_status, "{second}");
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "a re-sent transaction commits nothing"
    );
    let first_location = first["entry"][0]["response"]["location"].as_str();
    assert!(first_location.is_some(), "{first}");
    assert_eq!(
        first_location,
        second["entry"][0]["response"]["location"].as_str(),
        "the re-sent transaction names the resource its first delivery created: {second}"
    );
    assert_eq!(
        Some("200 OK"),
        second["entry"][0]["response"]["status"].as_str()
    );
    Ok(())
}

/// How long the stub CDR holds a commit answer in the overlap cases.
const HELD: Duration = Duration::from_millis(800);

/// How long the second delivery of an overlap case waits before it starts,
/// well inside [`HELD`].
const STAGGER: Duration = Duration::from_millis(150);

/// Returns the `location` of every issue of an `OperationOutcome` body.
fn issue_locations(body: &serde_json::Value) -> Vec<&str> {
    body["issue"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|issue| issue["location"][0].as_str())
        .collect()
}

#[tokio::test]
async fn two_overlapping_deliveries_of_one_transaction_commit_once() -> Result<(), Box<dyn StdError>>
{
    // No specification governs the redelivery rule: our own design. The CDR
    // holds the first commit's answer, so the second delivery starts while the
    // first holds its claim, and is refused with nothing committed.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo_after(&harness.cdr, ECHOED, Echo::Sent, HELD).await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition())]);
    let (first, second) = tokio::join!(post_transaction(&harness, &bundle), async {
        tokio::time::sleep(STAGGER).await;
        post_transaction(&harness, &bundle).await
    });
    let (first_status, first) = first?;
    let (second_status, second) = second?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    assert_eq!(
        Some("201 Created"),
        first["entry"][0]["response"]["status"].as_str(),
        "{first}"
    );
    assert_eq!(StatusCode::CONFLICT, second_status, "{second}");
    assert_eq!(Some("duplicate"), first_issue(&second)["code"].as_str());
    assert_eq!(
        vec!["urn:uuid:0000-good"],
        issue_locations(&second),
        "{second}"
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the overlapping delivery commits nothing"
    );

    let (third_status, third) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, third_status, "{third}");
    assert_eq!(
        Some("200 OK"),
        third["entry"][0]["response"]["status"].as_str(),
        "sent after the first delivery settled, the Bundle answers as re-sent: {third}"
    );
    assert_eq!(
        first["entry"][0]["response"]["location"].as_str(),
        third["entry"][0]["response"]["location"].as_str(),
        "{third}"
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the delivery after the first settled commits nothing either"
    );
    Ok(())
}

#[tokio::test]
async fn two_overlapping_creates_of_one_resource_commit_once() -> Result<(), Box<dyn StdError>> {
    // No specification governs the redelivery rule: our own design. The CDR
    // holds the first create's answer while the second create of the same
    // `id` and `meta.versionId` arrives.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", format!("W/\"{VERSION_ONE}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(harness.composition().to_string())
                .set_delay(HELD),
        )
        .mount(&harness.cdr)
        .await;
    let (first, second) = tokio::join!(call(harness.app(), post_condition(&condition())), async {
        tokio::time::sleep(STAGGER).await;
        call(harness.app(), post_condition(&condition())).await
    });
    let (first_status, first) = first?;
    let (second_status, second) = second?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    assert_eq!(StatusCode::CONFLICT, second_status, "{second}");
    assert_eq!(Some("duplicate"), first_issue(&second)["code"].as_str());
    assert_eq!(
        vec!["Condition/ferrobridge-synthetic-condition-1"],
        issue_locations(&second),
        "{second}"
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "the overlapping create commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_partly_consumed_transaction_is_refused_naming_the_consumed_entries()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-old", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let mut fresh = condition();
    fresh["id"] = serde_json::json!("ferrobridge-synthetic-condition-2");
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-old", condition()),
            ("urn:uuid:0000-new", fresh),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::CONFLICT, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    let named: Vec<&str> = body["issue"]
        .as_array()
        .ok_or("the refusal carries issues")?
        .iter()
        .filter_map(|issue| issue["location"][0].as_str())
        .collect();
    assert_eq!(vec!["urn:uuid:0000-old"], named, "{body}");
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "a refused transaction commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_transaction_carrying_one_resource_twice_is_refused() -> Result<(), Box<dyn StdError>> {
    // "A resource can only appear in a transaction once (by identity)"
    // (<https://hl7.org/fhir/R4/http.html#transaction>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-one", condition()),
            ("urn:uuid:0000-two", condition()),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    assert_eq!(
        Some("urn:uuid:0000-two"),
        first_issue(&body)["location"][0].as_str()
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await
    );
    Ok(())
}

#[tokio::test]
async fn a_transaction_carrying_one_id_at_two_versions_is_refused_naming_both()
-> Result<(), Box<dyn StdError>> {
    // "A resource can only appear in a transaction once (by identity)", and
    // an overlap of identities fails the transaction
    // (<https://hl7.org/fhir/R4/http.html#transaction>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-one", condition_at("1")),
            ("urn:uuid:0000-two", condition_at("2")),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status, "{body}");
    let issue = first_issue(&body);
    assert_eq!(Some("invalid"), issue["code"].as_str());
    assert_eq!(
        Some(vec!["urn:uuid:0000-one", "urn:uuid:0000-two"]),
        issue["location"].as_array().map(|locations| locations
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect()),
        "{body}"
    );
    assert_eq!((0, 0, 0), writes(&harness).await, "nothing is committed");
    assert_eq!(None, harness.store.consumed(&condition_source_at("1")?)?);
    Ok(())
}

#[tokio::test]
async fn a_batch_bundle_is_not_supported_in_this_milestone() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    // FHIR JSON forbids an empty array (<https://hl7.org/fhir/R4/json.html>),
    // so a batch Bundle with no entry omits the element rather than sending
    // one the strict codec would refuse before the batch check runs.
    let bundle = serde_json::json!({ "resourceType": "Bundle", "type": "batch" });
    let request = Request::post("/fhir")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(bundle.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
    Ok(())
}
