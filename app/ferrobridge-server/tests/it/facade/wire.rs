// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The wire rules every interaction shares: the media types, the refusals
//! of a malformed request, the CDR statuses through the status table, and
//! the error shape.

use crate::support;
use axum::body::Body;
use ferrobridge_server::facade::ehr::Policy;
use http::Request;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::EHR_ID;
use super::Harness;
use super::VALIDATE;
use super::VERSION_ONE;
use super::VERSION_TWO;
use super::assert_nothing_written;
use super::call;
use super::condition;
use super::diagnostics_of;
use super::first_issue;
use super::harness;
use super::harness_with;
use super::post_condition;
use super::raw;
use super::settings_with;
use super::stub::Echo;
use super::stub::mount_create;
use super::stub::mount_echo;
use super::stub::mount_ehr;
use super::stub::mount_no_ehr;
use super::stub::mount_read;
use super::stub::mount_update;

/// The containers the echo CDR hands out beside a create that took
/// [`CONTAINER`].
const ECHOED_BESIDE_CREATE: &[&str] = &["5f0e2b1a-0000-4000-8000-00000000000d"];

#[tokio::test]
async fn a_body_media_type_this_server_does_not_read_is_four_hundred_and_fifteen()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+xml")
        .body(Body::from("<Condition/>"))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::UNSUPPORTED_MEDIA_TYPE, status);
    assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_malformed_count_is_four_hundred_invalid() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/metadata?_count=many").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(
        Some("invalid"),
        first_issue(&body)["code"].as_str(),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_property_is_four_hundred_structure() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut resource = condition();
    resource["notAnElement"] = serde_json::json!("x");
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(
        Some("structure"),
        first_issue(&body)["code"].as_str(),
        "an unknown property is refused, never dropped: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_resource_no_program_claims_is_four_hundred_and_twenty_two()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut resource = condition();
    resource["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    let issue = first_issue(&body);
    assert_eq!(Some("not-supported"), issue["code"].as_str());
    assert!(
        issue["diagnostics"]
            .as_str()
            .is_some_and(|text| text.contains("http://example.org/other")),
        "the refusal names the profiles it saw: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_validation_refusal_is_four_hundred_and_twenty_two_with_its_errors()
-> Result<(), Box<dyn StdError>> {
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
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .expect("the refusal carries the CDR's own words");
    assert!(diagnostics.contains("the composition is invalid"), "{body}");
    assert!(
        diagnostics.contains("/content[0] is required"),
        "the validationErrors travel verbatim: {body}"
    );
    assert!(
        !body.to_string().contains("\"validationErrors\""),
        "the openEHR error body never reaches the wire raw: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_five_hundred_is_five_hundred_and_two_carrying_the_upstream_status()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(ResponseTemplate::new(503))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::BAD_GATEWAY, status);
    assert!(
        first_issue(&body)["diagnostics"]
            .as_str()
            .is_some_and(|text| text.contains("503")),
        "the 502 names the upstream status: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_four_hundred_and_one_stays_four_hundred_and_one_with_its_challenge()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Bearer realm=\"cdr\""),
        )
        .mount(&harness.cdr)
        .await;
    let response = raw(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::UNAUTHORIZED, response.status());
    assert_ne!(
        StatusCode::FORBIDDEN,
        response.status(),
        "a CDR 401 never becomes a 403"
    );
    assert_eq!(
        Some("Bearer realm=\"cdr\""),
        response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_status_its_rest_does_not_document_is_five_hundred() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(ResponseTemplate::new(415))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(
        StatusCode::INTERNAL_SERVER_ERROR,
        status,
        "a 415 from the CDR means the bridge chose a call it does not offer: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn the_three_absences_differ_on_the_wire() -> Result<(), Box<dyn StdError>> {
    // The carry-over case of #115: a disabled surface, a type outside the
    // loaded programs and an id of a loaded type that nothing maps are three
    // answers.
    let harness = harness().await;
    let disabled = call(
        ferrobridge_server::router(support::state(), &support::settings()),
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
    )
    .await?;
    let outside = call(
        harness.app(),
        Request::get("/fhir/Observation/abcdef").body(Body::empty())?,
    )
    .await?;
    let unmapped = call(
        harness.app(),
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
    )
    .await?;
    for (status, _) in [&disabled, &outside, &unmapped] {
        assert_eq!(StatusCode::NOT_FOUND, *status);
    }
    assert_eq!(
        Some("not-supported"),
        first_issue(&outside.1)["code"].as_str()
    );
    assert_eq!(Some("not-found"), first_issue(&unmapped.1)["code"].as_str());
    let codes = [
        disabled.1["issue"][0]["code"].clone(),
        outside.1["issue"][0]["code"].clone(),
        unmapped.1["issue"][0]["code"].clone(),
    ];
    let diagnostics = [
        disabled.1["issue"][0]["diagnostics"].clone(),
        outside.1["issue"][0]["diagnostics"].clone(),
        unmapped.1["issue"][0]["diagnostics"].clone(),
    ];
    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(codes[left], codes[right], "{codes:?}");
        assert_ne!(diagnostics[left], diagnostics[right], "{diagnostics:?}");
    }
    Ok(())
}

#[tokio::test]
async fn both_json_media_types_are_read_on_every_write() -> Result<(), Box<dyn StdError>> {
    // "The MIME-type for JSON content is `application/fhir+json`" and
    // `application/json` is its alias (<https://hl7.org/fhir/R4/http.html#mime-type>).
    for media in ["application/fhir+json", "application/json"] {
        let harness = harness().await;
        let id = create_one_as(&harness, media).await?;
        mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
        mount_update(
            &harness.cdr,
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(harness.composition().to_string()),
        )
        .await;
        let updated = call(
            harness.app(),
            Request::put(format!("/fhir/Condition/{id}"))
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(condition().to_string()))?,
        )
        .await?;
        assert_eq!(
            StatusCode::OK,
            updated.0,
            "update as {media}: {}",
            updated.1
        );
        let validated = call(
            harness.app(),
            Request::post(VALIDATE)
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(condition().to_string()))?,
        )
        .await?;
        assert_eq!(
            StatusCode::OK,
            validated.0,
            "$validate as {media}: {}",
            validated.1
        );
        mount_echo(&harness.cdr, ECHOED_BESIDE_CREATE, Echo::Sent).await;
        let mut fresh = condition();
        fresh["id"] = serde_json::json!("a-transaction-entry");
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                { "fullUrl": "urn:uuid:0000-good", "resource": fresh,
                  "request": { "method": "POST", "url": "Condition" } }
            ]
        });
        let transaction = call(
            harness.app(),
            Request::post("/fhir")
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(bundle.to_string()))?,
        )
        .await?;
        assert_eq!(
            StatusCode::OK,
            transaction.0,
            "transaction as {media}: {}",
            transaction.1
        );
    }
    Ok(())
}

#[tokio::test]
async fn any_other_media_type_is_four_hundred_and_fifteen_on_every_write()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    for media in ["application/fhir+xml", "text/plain", "application/xml"] {
        for request in [
            Request::post("/fhir/Condition"),
            Request::put("/fhir/Condition/abcdef"),
            Request::post(VALIDATE),
            Request::post("/fhir"),
        ] {
            let (status, body) = call(
                harness.app(),
                request
                    .header(header::CONTENT_TYPE, media)
                    .body(Body::from(condition().to_string()))?,
            )
            .await?;
            assert_eq!(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                status,
                "{media}: {body}"
            );
            assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
        }
    }
    assert_nothing_written(&harness).await;
    Ok(())
}

/// Creates the fixture Condition sent as `media` and returns its logical id.
async fn create_one_as(harness: &Harness, media: &str) -> Result<String, Box<dyn StdError>> {
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(
        harness.app(),
        Request::post("/fhir/Condition")
            .header(header::CONTENT_TYPE, media)
            .body(Body::from(condition().to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::CREATED, status, "create as {media}: {body}");
    Ok(body["id"]
        .as_str()
        .ok_or("the created resource carries an id")?
        .to_owned())
}

#[tokio::test]
async fn every_answer_carries_the_fhir_media_type() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    for request in [
        Request::get("/fhir/metadata").body(Body::empty())?,
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
        Request::get("/fhir/Observation/abcdef").body(Body::empty())?,
    ] {
        let response = raw(harness.app(), request).await?;
        assert_eq!(
            Some("application/fhir+json"),
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            "{:?}",
            response.status()
        );
    }
    Ok(())
}

/// An error body member no openEHR error document defines, which a rendering
/// of the client's outcome would carry onto the wire.
const UNDOCUMENTED_MEMBER: &str = "ferrobridge-stack-trace-0001";

/// Returns a CDR error body with a `message` and a member outside the
/// `{error, message, validationErrors}` shape.
fn leaky_error_body(message: &str) -> String {
    serde_json::json!({ "message": message, "trace": UNDOCUMENTED_MEMBER }).to_string()
}

#[tokio::test]
async fn a_refused_ehr_creation_carries_only_the_documented_error_shape()
-> Result<(), Box<dyn StdError>> {
    let harness = harness_with(settings_with(Policy::CreateOnFirstWrite)).await;
    mount_no_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/ehr"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("Content-Type", "application/json")
                .set_body_string(leaky_error_body("an EHR for the subject exists")),
        )
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    let diagnostics = diagnostics_of(&body).join(" ");
    assert!(
        diagnostics.contains("an EHR for the subject exists"),
        "the CDR's message travels in the diagnostics: {body}"
    );
    assert!(
        !body.to_string().contains(UNDOCUMENTED_MEMBER),
        "a member outside the openEHR error shape stays off the wire: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_composition_carries_only_the_documented_error_shape()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("Content-Type", "application/json")
                .set_body_string(leaky_error_body("the composition is malformed")),
        )
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    assert!(
        diagnostics_of(&body)
            .join(" ")
            .contains("the composition is malformed"),
        "{body}"
    );
    assert!(
        !body.to_string().contains(UNDOCUMENTED_MEMBER),
        "a member outside the openEHR error shape stays off the wire: {body}"
    );
    Ok(())
}
