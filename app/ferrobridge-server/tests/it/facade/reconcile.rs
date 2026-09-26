// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding of a committed contribution's versions to the entries, and
//! the retry after a binding failed.

use axum::body::Body;
use ferrobridge_server::facade::identity::store::Store;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;
use wiremock::Mock;
use wiremock::matchers;

use super::BASE_URL;
use super::CONTAINER;
use super::CONTRIBUTION;
use super::EHR_ID;
use super::Harness;
use super::VERSION_ONE;
use super::VERSION_TWO;
use super::call;
use super::condition;
use super::condition_at;
use super::condition_source_at;
use super::first_issue;
use super::harness;
use super::post_condition;
use super::post_transaction;
use super::raw;
use super::stub::Committed;
use super::stub::ECHOED;
use super::stub::Echo;
use super::stub::contribution_created;
use super::stub::mount_echo;
use super::stub::mount_echo_tracked;
use super::stub::mount_ehr;
use super::transaction_of;

#[tokio::test]
async fn a_contribution_answer_naming_no_versions_for_the_entries_is_a_typed_failure()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(contribution_created(
            CONTRIBUTION,
            &[VERSION_ONE, VERSION_TWO],
        ))
        .mount(&harness.cdr)
        .await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .unwrap_or_default();
    assert!(
        diagnostics.contains(CONTRIBUTION),
        "the failure names the committed contribution: {body}"
    );
    Ok(())
}

/// Returns the container the identity map bound the entry `location` names
/// to.
fn bound_container(harness: &Harness, location: Option<&str>) -> Result<String, Box<dyn StdError>> {
    let location = location.ok_or("the entry answers a location")?;
    let id = location
        .strip_prefix(&format!("{BASE_URL}/Condition/"))
        .and_then(|rest| rest.split('/').next())
        .ok_or(format!("{location} is no Condition location"))?;
    let internal = ferrobridge_server::facade::identity::FhirResourceId::new(id)?;
    let binding = harness
        .store
        .binding_of("Condition", &internal)?
        .ok_or("the entry's binding is recorded")?;
    Ok(binding.versioned_object_uid)
}

#[tokio::test]
async fn a_reversed_version_list_still_binds_each_entry_to_its_own_composition()
-> Result<(), Box<dyn StdError>> {
    // ITS-REST 1.1.0 states no order for `CONTRIBUTION.versions`, so each
    // version is matched to its entry by the FEEDER_AUDIT read back.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Reversed).await;
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
    let first_bound = bound_container(&harness, body["entry"][0]["response"]["location"].as_str())?;
    let second_bound =
        bound_container(&harness, body["entry"][1]["response"]["location"].as_str())?;
    let echoed_first = ECHOED.first().copied().ok_or("a first container")?;
    let echoed_second = ECHOED.get(1).copied().ok_or("a second container")?;
    assert_eq!(
        echoed_first, first_bound,
        "the first entry's own composition"
    );
    assert_eq!(
        echoed_second, second_bound,
        "the second entry's own composition"
    );
    Ok(())
}

#[tokio::test]
async fn a_committed_version_that_matches_no_entry_binds_nothing() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Foreign).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .unwrap_or_default();
    assert!(
        diagnostics.contains(CONTRIBUTION) && diagnostics.contains("matches 0"),
        "the failure names the contribution and the unmatched version: {body}"
    );
    let source = ferrobridge_server::facade::identity::record::SourceVersion::new(
        "Condition",
        ferrobridge_server::facade::identity::ExternalResourceId::new(
            "ferrobridge-synthetic-condition-1",
        )?,
        None,
    );
    assert_eq!(
        None,
        harness.store.consumed(&source)?,
        "an unverified binding records no consumed source"
    );
    Ok(())
}

/// Returns the consumed-source key of the synthetic Condition.
fn condition_source()
-> Result<ferrobridge_server::facade::identity::record::SourceVersion, Box<dyn StdError>> {
    Ok(
        ferrobridge_server::facade::identity::record::SourceVersion::new(
            "Condition",
            ferrobridge_server::facade::identity::ExternalResourceId::new(
                "ferrobridge-synthetic-condition-1",
            )?,
            None,
        ),
    )
}

#[tokio::test]
async fn a_commit_answered_without_its_versions_binds_from_the_contribution_read_back()
-> Result<(), Box<dyn StdError>> {
    // An empty `201` names the contribution in its `ETag`
    // (`ehr-codegen.openapi.yaml`, `201_CONTRIBUTION`), and `contribution_get`
    // answers the CONTRIBUTION whose `versions` the entries bind from.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Minimal).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("201 Created"), response["status"].as_str(), "{body}");
    assert_eq!(Some("W/\"1\""), response["etag"].as_str());
    assert_eq!(
        CONTAINER,
        bound_container(&harness, response["location"].as_str())?
    );
    assert_eq!(
        1,
        harness
            .received("GET", &format!("/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"))
            .await,
        "the versions are read back once"
    );
    assert!(harness.store.consumed(&condition_source()?)?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_contribution_that_cannot_be_read_back_is_a_typed_failure_and_its_retry_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "the contribution is not readable yet",
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.cdr)
        .await;
    mount_echo(&harness.cdr, ECHOED, Echo::Minimal).await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition())]);

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .unwrap_or_default();
    assert!(
        diagnostics.contains(CONTRIBUTION),
        "the failure names the committed contribution: {body}"
    );
    let source = condition_source()?;
    assert_eq!(None, harness.store.consumed(&source)?);
    let committed = harness
        .store
        .committed(&source)?
        .ok_or("the commit is recorded before the binding")?;
    assert_eq!(CONTRIBUTION, committed.contribution_uid);

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("200 OK"), response["status"].as_str(), "{body}");
    assert_eq!(
        CONTAINER,
        bound_container(&harness, response["location"].as_str())?
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the retry commits nothing"
    );
    assert!(harness.store.consumed(&source)?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_single_create_of_a_committed_unbound_entry_binds_from_the_read_back_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "the contribution is not readable yet",
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.cdr)
        .await;
    mount_echo(&harness.cdr, ECHOED, Echo::Minimal).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let source = condition_source()?;
    assert_eq!(None, harness.store.consumed(&source)?);
    assert!(harness.store.committed(&source)?.is_some());

    let response = raw(harness.app(), post_condition(&condition())).await?;
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    assert_eq!(
        StatusCode::OK,
        status,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(CONTAINER, bound_container(&harness, location.as_deref())?);
    let rendered: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(Some("Condition"), rendered["resourceType"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "one contribution across both deliveries"
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "the single create commits nothing"
    );
    assert!(harness.store.consumed(&source)?.is_some());
    Ok(())
}

/// Returns how many version containers the echo CDR committed into.
fn containers_of(committed: &Arc<std::sync::Mutex<Committed>>) -> Result<usize, Box<dyn StdError>> {
    let held = committed
        .lock()
        .map_err(|_poisoned| "the echo CDR's record is poisoned")?;
    Ok(held.containers().len())
}

#[tokio::test]
async fn a_transaction_entry_at_a_new_version_id_of_a_known_resource_commits_one_modification()
-> Result<(), Box<dyn StdError>> {
    // A CONTRIBUTION version that follows another names it as its
    // `preceding_version_uid` (`ehr-codegen.openapi.yaml`, `UpdateVersion`),
    // and its audit states a modification (openEHR terminology code 251).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let committed = mount_echo_tracked(&harness.cdr, ECHOED, Echo::Sent, Duration::ZERO).await;
    let (first_status, first) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition_at("1"))]),
    )
    .await?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    let first_location = first["entry"][0]["response"]["location"]
        .as_str()
        .ok_or("the first delivery answers a location")?;
    let id = first_location
        .strip_prefix(&format!("{BASE_URL}/Condition/"))
        .and_then(|rest| rest.strip_suffix("/_history/1"))
        .ok_or(format!("{first_location} is no first version"))?
        .to_owned();

    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition_at("2"))]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("200 OK"), response["status"].as_str(), "{body}");
    assert_eq!(
        Some(format!("{BASE_URL}/Condition/{id}/_history/2").as_str()),
        response["location"].as_str(),
        "the same resource, one version on"
    );
    assert_eq!(Some("W/\"2\""), response["etag"].as_str());
    assert_eq!(
        2,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "one contribution per delivery"
    );
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/contribution"))
        .await
        .ok_or("the second contribution was sent")?;
    let versions = sent["versions"]
        .as_array()
        .ok_or("the contribution carries versions")?;
    assert_eq!(1, versions.len(), "one modification: {sent}");
    let version = versions.first().ok_or("one version")?;
    assert_eq!(
        Some(VERSION_ONE),
        version["preceding_version_uid"]["value"].as_str(),
        "the modification follows the version the CDR holds: {sent}"
    );
    assert_eq!(
        Some("251"),
        version["commit_audit"]["change_type"]["defining_code"]["code_string"].as_str(),
        "{sent}"
    );
    assert_eq!(1, containers_of(&committed)?, "no second composition");
    assert_eq!(
        Some(CONTAINER),
        harness
            .store
            .consumed(&condition_source_at("2")?)?
            .as_ref()
            .map(|known| known.versioned_object_uid.as_str()),
        "the new version is recorded against the same composition"
    );
    let (status, read) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{read}");
    assert_eq!(Some(id.as_str()), read["id"].as_str());
    assert_eq!(Some("2"), read["meta"]["versionId"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_revising_transaction_entry_whose_binding_failed_binds_on_retry_and_commits_once()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let committed = mount_echo_tracked(&harness.cdr, ECHOED, Echo::Minimal, Duration::ZERO).await;
    let (first_status, first) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition_at("1"))]),
    )
    .await?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "the contribution is not readable yet",
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.cdr)
        .await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition_at("2"))]);

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let source = condition_source_at("2")?;
    assert_eq!(None, harness.store.consumed(&source)?);
    assert!(
        harness.store.committed(&source)?.is_some(),
        "the commit is recorded before the binding"
    );

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("200 OK"), response["status"].as_str(), "{body}");
    assert_eq!(Some("W/\"2\""), response["etag"].as_str(), "{body}");
    assert_eq!(
        CONTAINER,
        bound_container(&harness, response["location"].as_str())?
    );
    assert_eq!(
        2,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the retry commits nothing"
    );
    assert_eq!(1, containers_of(&committed)?, "no second composition");
    assert_eq!(
        Some(CONTAINER),
        harness
            .store
            .consumed(&source)?
            .as_ref()
            .map(|known| known.versioned_object_uid.as_str())
    );
    Ok(())
}
