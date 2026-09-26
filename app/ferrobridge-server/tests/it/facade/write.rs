// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The single create and update: the identity they record, the feeder audit,
//! the EHR resolved or created by subject, and the `If-Match` rule.

use axum::body::Body;
use ferrobridge_server::facade::ehr::Policy;
use ferrobridge_server::facade::identity::store::Store;
use http::Request;
use http::Response;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::BASE_URL;
use super::CONTAINER;
use super::EHR_ID;
use super::VERSION_ONE;
use super::VERSION_TWO;
use super::call;
use super::condition;
use super::create_one;
use super::first_issue;
use super::get_tagged;
use super::harness;
use super::harness_with;
use super::post_condition;
use super::raw;
use super::revised_condition;
use super::settings_with;
use super::stub::mount_create;
use super::stub::mount_ehr;
use super::stub::mount_no_ehr;
use super::stub::mount_read;
use super::stub::mount_update;
use super::stub::mount_version_two;
use super::writes;

#[tokio::test]
async fn a_create_answers_two_hundred_and_one_with_location_and_etag()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let response = raw(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::CREATED, response.status());
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("a create carries Location")
        .to_owned();
    assert_eq!(
        Some("W/\"1\""),
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let id = body["id"].as_str().expect("the resource carries an id");
    assert_eq!(
        format!("{BASE_URL}/Condition/{id}/_history/1"),
        location,
        "Location names the FHIR resource, never the composition"
    );
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert!(
        body["meta"]["source"]
            .as_str()
            .is_some_and(|source| source.contains(VERSION_ONE)),
        "meta.source names the openEHR version uid: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_records_the_identity_it_assigned() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    let internal = ferrobridge_server::facade::identity::FhirResourceId::new(&id)?;
    let binding = harness
        .store
        .binding_of("Condition", &internal)?
        .expect("the create recorded where the resource lives");
    assert_eq!(EHR_ID, binding.ehr_id);
    assert_eq!(CONTAINER, binding.versioned_object_uid);
    assert_eq!("ferrobridge_facade.context", binding.context);
    Ok(())
}

#[tokio::test]
async fn a_create_records_the_source_resource_in_the_feeder_audit() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    create_one(&harness).await?;
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/composition"))
        .await
        .ok_or("the create sent a composition")?;
    let audit = &sent["feeder_audit"];
    let item = &audit["originating_system_item_ids"][0];
    assert_eq!(
        Some("ferrobridge-synthetic-condition-1"),
        item["id"].as_str(),
        "the source resource id travels in the feeder audit: {audit}"
    );
    assert_eq!(Some("Condition"), item["type"].as_str());
    assert_eq!(
        Some("ferrobridge.test"),
        audit["originating_system_audit"]["system_id"].as_str(),
        "the feeder audit names this bridge: {audit}"
    );
    Ok(())
}

#[tokio::test]
async fn a_derived_id_follows_the_r4_grammar() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    assert!(
        ferrobridge_server::facade::identity::is_fhir_id(&id),
        "{id} is outside [A-Za-z0-9\\-\\.]{{1,64}}"
    );
    Ok(())
}

#[tokio::test]
async fn an_update_of_an_id_the_map_does_not_know_is_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let request = Request::put("/fhir/Condition/abcdef")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(
        StatusCode::NOT_FOUND,
        status,
        "updateCreate is false, so a PUT to an unknown id creates nothing"
    );
    assert_eq!(Some("not-found"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn an_update_without_if_match_checks_the_current_etag() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(harness.composition().to_string()),
    )
    .await;
    let request = Request::put(format!("/fhir/Condition/{id}"))
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert!(
        harness
            .received("GET", &format!("/ehr/{EHR_ID}/composition/{CONTAINER}"))
            .await
            >= 1,
        "a PUT without If-Match reads the CDR's current ETag first"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_precondition_failure_is_four_hundred_and_twelve_with_the_current_etag()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(412).insert_header("ETag", format!("W/\"{VERSION_TWO}\"")),
    )
    .await;
    let request = Request::put(format!("/fhir/Condition/{id}"))
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header(header::IF_MATCH, format!("W/\"{VERSION_ONE}\""))
        .body(Body::from(condition().to_string()))?;
    let response = raw(harness.app(), request).await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, response.status());
    assert_eq!(
        Some("W/\"2\""),
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        "the 412 carries the version the CDR reports as current"
    );
    Ok(())
}

/// Returns a `PUT [base]/Condition/{id}` carrying `If-Match: {if_match}`.
fn put_if_match(id: &str, if_match: &str) -> Result<Request<Body>, Box<dyn StdError>> {
    Ok(Request::put(format!("/fhir/Condition/{id}"))
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header(header::IF_MATCH, if_match)
        .body(Body::from(revised_condition().to_string()))?)
}

/// Returns one response header as text.
fn header_text<'a>(response: &'a Response<Body>, name: &header::HeaderName) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn an_update_carrying_the_etag_a_read_answered_commits_the_next_version()
-> Result<(), Box<dyn StdError>> {
    // The ETag is the versionId as a weak entity tag, and a version-aware
    // update sends it back in If-Match
    // (<https://hl7.org/fhir/R4/http.html#concurrency>).
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, tag, read) = get_tagged(&harness, &format!("/fhir/Condition/{id}")).await?;
    assert_eq!(StatusCode::OK, status, "{read}");
    let tag = tag.ok_or("a read carries an ETag")?;
    assert_eq!("W/\"1\"", tag);
    Mock::given(matchers::method("PUT"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .and(matchers::header("If-Match", format!("\"{VERSION_ONE}\"")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(harness.composition().to_string()),
        )
        .mount(&harness.cdr)
        .await;
    let response = raw(harness.app(), put_if_match(&id, &tag)?).await?;
    assert_eq!(StatusCode::OK, response.status());
    assert_eq!(Some("W/\"2\""), header_text(&response, &header::ETAG));
    assert!(
        header_text(&response, &header::LOCATION)
            .is_some_and(|location| location.ends_with(&format!("/Condition/{id}/_history/2"))),
        "{:?}",
        response.headers()
    );
    assert_eq!(
        1,
        writes(&harness).await.1,
        "the CDR received the completed version"
    );
    Ok(())
}

#[tokio::test]
async fn an_update_carrying_a_quoted_or_bare_version_id_is_completed_the_same_way()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    for if_match in ["\"1\"", "1"] {
        let response = raw(harness.app(), put_if_match(&id, if_match)?).await?;
        assert_eq!(StatusCode::OK, response.status(), "If-Match: {if_match}");
        assert_eq!(Some("W/\"2\""), header_text(&response, &header::ETAG));
    }
    Ok(())
}

#[tokio::test]
async fn an_update_carrying_a_stale_etag_is_four_hundred_and_twelve_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    // A version-aware update whose If-Match does not name the current version
    // is 412 (<https://hl7.org/fhir/R4/http.html#concurrency>).
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_TWO, &harness.composition()).await;
    mount_version_two(&harness).await;
    let response = raw(harness.app(), put_if_match(&id, "W/\"1\"")?).await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, response.status());
    assert_eq!(
        Some("W/\"2\""),
        header_text(&response, &header::ETAG),
        "the 412 carries the current version"
    );
    assert_eq!(0, writes(&harness).await.1, "no update reached the CDR");
    Ok(())
}

#[tokio::test]
async fn an_update_carrying_the_cdr_version_id_passes_it_through() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    let id = create_one(&harness).await?;
    Mock::given(matchers::method("PUT"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .and(matchers::header("If-Match", format!("\"{VERSION_ONE}\"")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(harness.composition().to_string()),
        )
        .mount(&harness.cdr)
        .await;
    let response = raw(
        harness.app(),
        put_if_match(&id, &format!("W/\"{VERSION_ONE}\""))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, response.status());
    assert_eq!(Some("W/\"2\""), header_text(&response, &header::ETAG));
    assert_eq!(
        0,
        harness
            .received("GET", &format!("/ehr/{EHR_ID}/composition/{CONTAINER}"))
            .await,
        "the CDR checks its own version id, so the facade reads nothing first"
    );
    Ok(())
}

#[tokio::test]
async fn an_update_carrying_a_version_of_another_composition_is_four_hundred_and_twelve()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_version_two(&harness).await;
    let foreign = "5f0e2b1a-0000-4000-8000-00000000000d::ferrobridge.test::1";
    let (status, body) = call(
        harness.app(),
        put_if_match(&id, &format!("W/\"{foreign}\""))?,
    )
    .await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, status, "{body}");
    assert_eq!(Some("conflict"), first_issue(&body)["code"].as_str());
    assert_eq!(0, writes(&harness).await.1, "no update reached the CDR");
    Ok(())
}

#[tokio::test]
async fn an_update_carrying_a_malformed_if_match_is_four_hundred_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    for if_match in ["W/\"one\"", "W/\"\"", "W/\"1"] {
        let (status, body) = call(harness.app(), put_if_match(&id, if_match)?).await?;
        assert_eq!(
            StatusCode::BAD_REQUEST,
            status,
            "If-Match: {if_match}: {body}"
        );
        assert_eq!(Some("invalid"), first_issue(&body)["code"].as_str());
    }
    assert_eq!(0, writes(&harness).await.1, "no update reached the CDR");
    Ok(())
}

#[tokio::test]
async fn invalid_mapped_content_stores_nothing() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(ferrobridge_testkit::stubs::its_rest::unprocessable(
            "the composition is invalid",
            &["/content[0] is required"],
        ))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    let source = ferrobridge_server::facade::identity::ExternalResourceId::new(
        "ferrobridge-synthetic-condition-1",
    )?;
    assert!(
        harness
            .store
            .consumed(
                &ferrobridge_server::facade::identity::record::SourceVersion::new(
                    "Condition",
                    source.clone(),
                    None,
                )
            )?
            .is_none(),
        "a refused commit recorded the source as consumed"
    );
    if let Some(internal) = harness.store.internal_of("Condition", &source)? {
        assert!(
            harness.store.binding_of("Condition", &internal)?.is_none(),
            "a refused commit bound the resource to a composition"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_feeder_audit_carries_the_source_version() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let mut resource = condition();
    resource["meta"]["versionId"] = serde_json::json!("7");
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/composition"))
        .await
        .ok_or("the create sent a composition")?;
    let audit = &sent["feeder_audit"]["originating_system_audit"];
    assert_eq!(
        Some("7"),
        audit["version_id"].as_str(),
        "meta.versionId travels as the source version: {audit}"
    );
    assert_eq!(Some("ferrobridge.test"), audit["system_id"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_source_resource_with_no_id_is_audited_as_unknown() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let mut resource = condition();
    if let Some(object) = resource.as_object_mut() {
        object.remove("id");
    }
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/composition"))
        .await
        .ok_or("the create sent a composition")?;
    let item = &sent["feeder_audit"]["originating_system_item_ids"][0];
    assert_eq!(
        Some("unknown"),
        item["id"].as_str(),
        "an absent id is recorded as unknown, never invented: {item}"
    );
    assert_eq!(Some("Condition"), item["type"].as_str());
    Ok(())
}

#[tokio::test]
async fn an_ehr_created_on_first_sight_names_the_subject_in_the_namespace()
-> Result<(), Box<dyn StdError>> {
    // The subject shape is a PARTY_SELF over a PARTY_REF whose GENERIC_ID
    // scheme is the namespace, which is what `ehr_get_by_subject` matches on
    // (`ehr-codegen.openapi.yaml`).
    let harness = harness_with(settings_with(Policy::CreateOnFirstWrite)).await;
    mount_no_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/ehr"))
        .respond_with(ResponseTemplate::new(201).insert_header("ETag", format!("\"{EHR_ID}\"")))
        .mount(&harness.cdr)
        .await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    let request = harness
        .cdr
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|request| request.method.as_str() == "POST" && request.url.path() == "/ehr")
        .ok_or("the facade created an EHR")?;
    let created: serde_json::Value = serde_json::from_slice(&request.body)?;
    assert_eq!(Some("EHR_STATUS"), created["_type"].as_str(), "{created}");
    let subject = &created["subject"];
    assert_eq!(Some("PARTY_SELF"), subject["_type"].as_str(), "{subject}");
    let reference = &subject["external_ref"];
    assert_eq!(Some("PARTY_REF"), reference["_type"].as_str(), "{subject}");
    assert_eq!(
        Some("http://example.org/fhir/sid/ferrobridge-subject"),
        reference["namespace"].as_str()
    );
    assert_eq!(Some("GENERIC_ID"), reference["id"]["_type"].as_str());
    assert_eq!(
        Some("synthetic-subject-0001"),
        reference["id"]["value"].as_str()
    );
    assert_eq!(
        reference["namespace"], reference["id"]["scheme"],
        "the GENERIC_ID scheme is the namespace: {subject}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_subject_creates_no_ehr_under_the_default_policy()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_no_ehr(&harness.cdr).await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert!(
        status.is_client_error(),
        "an unknown subject under Policy::Existing is refused: {status} {body}"
    );
    assert_eq!(
        0,
        harness.received("POST", "/ehr").await,
        "the facade created an EHR the policy does not allow"
    );
    Ok(())
}
