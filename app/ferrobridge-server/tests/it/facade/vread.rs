// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The vread of one version by the `ETag` a write answered.

use axum::body::Body;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::EHR_ID;
use super::VERSION_ONE;
use super::VERSION_TWO;
use super::call;
use super::condition;
use super::create_one;
use super::first_issue;
use super::get_tagged;
use super::harness;
use super::post_located;
use super::revised_condition;
use super::stub::mount_create;
use super::stub::mount_ehr;
use super::stub::mount_read;
use super::stub::mount_version_two;

/// Mounts the read of the composition version `version` that answers `200`
/// with `body`.
async fn mount_version(cdr: &MockServer, version: &str, body: &serde_json::Value) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{version}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(body.to_string()),
        )
        .mount(cdr)
        .await;
}

#[tokio::test]
async fn a_vread_of_the_location_a_create_answers_is_that_version() -> Result<(), Box<dyn StdError>>
{
    // vread answers `GET [base]/[type]/[id]/_history/[vid]` with an ETag
    // naming the versionId (<https://hl7.org/fhir/R4/http.html#vread>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, location, created) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::CREATED, status, "{created}");
    let location = location.ok_or("a create carries Location")?;
    let path = location
        .strip_prefix("http://ferrobridge.invalid")
        .ok_or(format!("{location} is not under the base"))?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, tag, body) = get_tagged(&harness, path).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some("W/\"1\""), tag.as_deref());
    assert_eq!(created["id"], body["id"]);
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert!(
        body["meta"]["source"]
            .as_str()
            .is_some_and(|source| source.ends_with(VERSION_ONE)),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn after_an_update_each_version_reads_back_with_its_own_content()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (status, updated) = call(
        harness.app(),
        Request::put(format!("/fhir/Condition/{id}"))
            .header(header::CONTENT_TYPE, "application/fhir+json")
            .body(Body::from(revised_condition().to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{updated}");
    assert_eq!(Some("2"), updated["meta"]["versionId"].as_str());

    harness.cdr.reset().await;
    let revised = harness.composition_of(&revised_condition());
    mount_read(&harness.cdr, VERSION_TWO, &revised).await;
    mount_version(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, tag, first) =
        get_tagged(&harness, &format!("/fhir/Condition/{id}/_history/1")).await?;
    assert_eq!(StatusCode::OK, status, "{first}");
    assert_eq!(Some("W/\"1\""), tag.as_deref());
    assert_eq!(Some("1"), first["meta"]["versionId"].as_str());
    assert_eq!(
        condition()["code"]["text"],
        first["code"]["text"],
        "{first}"
    );
    let (status, tag, second) =
        get_tagged(&harness, &format!("/fhir/Condition/{id}/_history/2")).await?;
    assert_eq!(StatusCode::OK, status, "{second}");
    assert_eq!(Some("W/\"2\""), tag.as_deref());
    assert_eq!(Some("2"), second["meta"]["versionId"].as_str());
    assert_eq!(
        revised_condition()["code"]["text"],
        second["code"]["text"],
        "{second}"
    );
    assert_ne!(first["code"]["text"], second["code"]["text"]);
    Ok(())
}

#[tokio::test]
async fn a_vread_of_a_version_the_cdr_does_not_hold_is_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    for vid in ["7", "not-a-version"] {
        let (status, body) = call(
            harness.app(),
            Request::get(format!("/fhir/Condition/{id}/_history/{vid}")).body(Body::empty())?,
        )
        .await?;
        assert_eq!(StatusCode::NOT_FOUND, status, "{vid}: {body}");
        assert_eq!(Some("not-found"), first_issue(&body)["code"].as_str());
    }
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/Condition/abcdef/_history/1").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::NOT_FOUND, status, "{body}");
    Ok(())
}

#[tokio::test]
async fn a_vread_of_a_version_the_cdr_reports_deleted_is_four_hundred_and_ten()
-> Result<(), Box<dyn StdError>> {
    // "If the version referred to is actually one where the resource was
    // deleted, the server should return a 410"
    // (<https://hl7.org/fhir/R4/http.html#vread>).
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_TWO, &harness.composition()).await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{VERSION_ONE}"
        )))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}/_history/1")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::GONE, status, "{body}");
    assert_eq!(Some("deleted"), first_issue(&body)["code"].as_str());
    Ok(())
}
