// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The conditional create, `If-None-Exist`.

use axum::body::Body;
use ferrobridge_server::facade::identity::store::Store;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;

use super::EHR_ID;
use super::VERSION_ONE;
use super::call;
use super::condition;
use super::conditional_create;
use super::conditional_create_as;
use super::create_one;
use super::first_issue;
use super::harness;
use super::post_transaction;
use super::stub::ECHOED;
use super::stub::Echo;
use super::stub::mount_create;
use super::stub::mount_echo;
use super::stub::mount_ehr;
use super::stub::mount_read;
use super::transaction_of;
use super::writes;

#[tokio::test]
async fn if_none_exist_creates_when_nothing_matches() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", "_id=notrecordedyet")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    Ok(())
}

#[tokio::test]
async fn if_none_exist_answers_two_hundred_when_one_matches() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let mut fresh = condition();
    fresh["id"] = serde_json::json!("another-sender-id");
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", format!("_id={id}"))
        .body(Body::from(fresh.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some(id.as_str()), body["id"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "a conditional create that matched created nothing"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_answers_four_hundred_and_twelve_when_several_match()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let first = create_one(&harness).await?;
    let second = ferrobridge_server::facade::identity::FhirResourceId::new("secondmatch")?;
    harness.store.record_binding(
        "Condition",
        &second,
        &harness
            .store
            .binding_of(
                "Condition",
                &ferrobridge_server::facade::identity::FhirResourceId::new(&first)?,
            )?
            .expect("the first create recorded a binding"),
    )?;
    // A resource the map already consumed resolves to an update before any
    // conditional create runs, so this case sends a source id the map does
    // not know.
    let mut fresh = condition();
    fresh["id"] = serde_json::json!("a-sender-id-the-map-has-not-seen");
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", format!("_id={first},secondmatch"))
        .body(Body::from(fresh.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    Ok(())
}

/// The `Identifier.system` of the synthetic Condition's one identifier.
const CONDITION_IDENTIFIER_SYSTEM: &str = "http://example.org/fhir/sid/ferrobridge-condition";

/// The `Identifier.value` of the synthetic Condition's one identifier.
const CONDITION_IDENTIFIER_VALUE: &str = "synthetic-condition-0001";

#[tokio::test]
async fn if_none_exist_on_an_identifier_a_committed_resource_carries_answers_two_hundred_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    // "If the search finds one match, the server returns 200 OK"
    // (<https://hl7.org/fhir/R4/http.html#ccreate>); the token forms are
    // `[system]|[code]` and `[code]` (<https://hl7.org/fhir/R4/search.html#token>).
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    for search in [
        format!("identifier={CONDITION_IDENTIFIER_SYSTEM}|{CONDITION_IDENTIFIER_VALUE}"),
        format!("identifier={CONDITION_IDENTIFIER_VALUE}"),
        format!(
            "identifier=http%3A%2F%2Fexample.org%2Ffhir%2Fsid%2Fferrobridge-condition%7C{CONDITION_IDENTIFIER_VALUE}"
        ),
    ] {
        let (status, body) = call(harness.app(), conditional_create(&search)?).await?;
        assert_eq!(StatusCode::OK, status, "{search}: {body}");
        assert_eq!(Some(id.as_str()), body["id"].as_str(), "{search}");
    }
    assert_eq!(
        (1, 0, 0),
        writes(&harness).await,
        "the one create before the searches is the only commit"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_on_an_identifier_no_resource_carries_still_creates()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    create_one(&harness).await?;
    for (sender_id, search) in [
        (
            "sender-a",
            format!("identifier={CONDITION_IDENTIFIER_SYSTEM}|another-value"),
        ),
        (
            "sender-b",
            format!("identifier=http://example.org/another-system|{CONDITION_IDENTIFIER_VALUE}"),
        ),
        (
            "sender-c",
            format!("identifier=|{CONDITION_IDENTIFIER_VALUE}"),
        ),
    ] {
        let (status, body) =
            call(harness.app(), conditional_create_as(sender_id, &search)?).await?;
        assert_eq!(StatusCode::CREATED, status, "{search}: {body}");
    }
    assert_eq!(
        (4, 0, 0),
        writes(&harness).await,
        "each unmatched conditional create commits its composition"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_on_an_identifier_two_resources_carry_is_four_hundred_and_twelve()
-> Result<(), Box<dyn StdError>> {
    // "If it finds more than one match, the server returns 412 Precondition
    // Failed" (<https://hl7.org/fhir/R4/http.html#ccreate>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Sent).await;
    let mut second = condition();
    second["id"] = serde_json::json!("ferrobridge-synthetic-condition-2");
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-first", condition()),
            ("urn:uuid:0000-second", second),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let (status, body) = call(
        harness.app(),
        conditional_create(&format!(
            "identifier={CONDITION_IDENTIFIER_SYSTEM}|{CONDITION_IDENTIFIER_VALUE}"
        ))?,
    )
    .await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the refused conditional create commits nothing"
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_on_every_code_of_a_system_is_refused() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        conditional_create(&format!("identifier={CONDITION_IDENTIFIER_SYSTEM}|"))?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status, "{body}");
    assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
    assert_eq!((0, 0, 0), writes(&harness).await);
    Ok(())
}
