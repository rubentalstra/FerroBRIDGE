// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A single create delivered again: the same source commits nothing, a
//! later version of it commits a later version of its composition.

use ferrobridge_server::facade::identity::store::Store;
use http::StatusCode;
use std::error::Error as StdError;
use wiremock::ResponseTemplate;

use super::BASE_URL;
use super::CONTAINER;
use super::EHR_ID;
use super::VERSION_ONE;
use super::VERSION_TWO;
use super::call;
use super::condition;
use super::condition_at;
use super::create_one;
use super::first_issue;
use super::harness;
use super::post_condition;
use super::post_located;
use super::revised_condition;
use super::stub::mount_create;
use super::stub::mount_ehr;
use super::stub::mount_read;
use super::stub::mount_update;
use super::stub::mount_version_two;
use super::writes;

#[tokio::test]
async fn a_re_sent_resource_updates_the_same_composition() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(harness.composition().to_string()),
    )
    .await;
    // A re-sent body with no `meta.versionId` and the content the map already
    // holds is a replay, so this case re-sends the resource with its content
    // changed, which is the update.
    let (status, body) = call(harness.app(), post_condition(&revised_condition())).await?;
    assert_eq!(
        StatusCode::OK,
        status,
        "a re-sent resource updates rather than duplicates: {body}"
    );
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "the second send created no second composition"
    );
    assert_eq!(
        1,
        harness
            .received("PUT", &format!("/ehr/{EHR_ID}/composition/{CONTAINER}"))
            .await,
        "the second send committed one later version"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_re_sent_with_its_id_and_version_id_commits_nothing_and_names_the_first_composition()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, first_location, first) = post_located(&harness, &condition_at("1")).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, location, body) = post_located(&harness, &condition_at("1")).await?;
    assert_eq!(StatusCode::OK, status, "a replay answers 200 OK: {body}");
    assert!(first_location.is_some(), "{first}");
    assert_eq!(
        first_location, location,
        "the replay names the composition the first delivery produced"
    );
    assert_eq!(first["id"], body["id"]);
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert_eq!(
        (1, 0, 0),
        writes(&harness).await,
        "one composition POST across both deliveries, and no update or contribution"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_of_a_known_id_at_a_new_version_id_commits_a_later_version()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, _, first) = post_located(&harness, &condition_at("1")).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, location, body) = post_located(&harness, &condition_at("2")).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(first["id"], body["id"], "the same resource, one version on");
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    let id = first["id"]
        .as_str()
        .ok_or("the created resource carries an id")?;
    assert_eq!(
        Some(format!("{BASE_URL}/Condition/{id}/_history/2")),
        location
    );
    assert_eq!(
        (1, 1, 0),
        writes(&harness).await,
        "one composition POST and one update PUT"
    );
    let source = ferrobridge_server::facade::identity::record::SourceVersion::new(
        "Condition",
        ferrobridge_server::facade::identity::ExternalResourceId::new(
            "ferrobridge-synthetic-condition-1",
        )?,
        Some(String::from("2")),
    );
    assert_eq!(
        Some(CONTAINER),
        harness
            .store
            .consumed(&source)?
            .as_ref()
            .map(|known| known.versioned_object_uid.as_str()),
        "the new version is recorded against the same composition"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_re_sent_without_a_version_id_and_with_the_same_content_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, first_location, first) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, location, body) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(first_location, location);
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert_eq!(
        (1, 0, 0),
        writes(&harness).await,
        "the same content with no versionId is a replay"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_re_sent_without_a_version_id_and_with_other_content_commits_a_later_version()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, _, first) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, _, body) = post_located(&harness, &revised_condition()).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(first["id"], body["id"]);
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    assert_eq!(
        (1, 1, 0),
        writes(&harness).await,
        "other content with no versionId is the update"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_of_an_id_the_map_binds_to_two_compositions_is_refused_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let source = |version: &str| -> Result<_, Box<dyn StdError>> {
        Ok(
            ferrobridge_server::facade::identity::record::SourceVersion::new(
                "Condition",
                ferrobridge_server::facade::identity::ExternalResourceId::new(
                    "ferrobridge-synthetic-condition-1",
                )?,
                Some(String::from(version)),
            ),
        )
    };
    for (version, container) in [
        ("1", CONTAINER),
        ("2", "5f0e2b1a-0000-4000-8000-00000000000d"),
    ] {
        harness.store.record_consumed(
            &source(version)?,
            &ferrobridge_server::facade::identity::record::ConsumedSource {
                ehr_id: String::from(EHR_ID),
                versioned_object_uid: String::from(container),
                internal_id: String::from("abc"),
                context: String::from("ferrobridge_facade.context"),
            },
        )?;
    }
    let (status, _, body) = post_located(&harness, &condition_at("3")).await?;
    assert_eq!(StatusCode::CONFLICT, status, "{body}");
    assert_eq!(Some("conflict"), first_issue(&body)["code"].as_str());
    assert_eq!((0, 0, 0), writes(&harness).await, "nothing is committed");
    Ok(())
}
