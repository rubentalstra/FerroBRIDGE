// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The batch interaction: one `Bundle`, one entry per operation
//! (<https://hl7.org/fhir/R4/http.html#transaction>).

use crate::support::{RELEASES, client, release_path, request_body};
use ferrobridge_term::config::WireVersion;
use ferrobridge_term::error::Error;
use ferrobridge_term::outcome::{BatchOutcome, LookupOutcome, Request, ValidateOutcome};
use ferrobridge_testkit::fixtures;
use ferrobridge_testkit::stubs::terminology;
use std::error::Error as StdError;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mounts `response` as the one answer of the service base.
async fn mount(server: &MockServer, version: WireVersion, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(format!("/{}", release_path(version))))
        .respond_with(response)
        .mount(server)
        .await;
}

/// The three requests the suite sends, in order.
fn requests() -> Vec<Request> {
    vec![
        Request::Lookup {
            system: fixtures::TERM_SOURCE_SYSTEM.to_owned(),
            code: fixtures::TERM_MEMBER_CODE.to_owned(),
            version: None,
        },
        Request::Lookup {
            system: fixtures::TERM_SOURCE_SYSTEM.to_owned(),
            code: fixtures::TERM_UNKNOWN_CODE.to_owned(),
            version: None,
        },
        Request::ValidateCode {
            value_set_url: fixtures::TERM_VALUE_SET_URL.to_owned(),
            system: fixtures::TERM_SOURCE_SYSTEM.to_owned(),
            code: fixtures::TERM_MEMBER_CODE.to_owned(),
        },
    ]
}

/// The `batch-response` those three requests are answered with.
fn answers() -> ResponseTemplate {
    terminology::batch_response(&[
        (
            "200 OK",
            terminology::parameters_resource(&terminology::lookup_body(
                "FerroBridgeSyntheticSource",
                fixtures::TERM_MEMBER_DISPLAY,
                None,
                &[],
                &[],
            )),
        ),
        (
            "400 Bad Request",
            terminology::outcome_body(
                "code-invalid",
                Some("invalid-code"),
                "the code is not in the code system",
            ),
        ),
        (
            "200 OK",
            terminology::parameters_resource(&terminology::validate_code_body(
                true,
                Some(fixtures::TERM_MEMBER_DISPLAY),
                None,
                None,
            )),
        ),
    ])
}

#[tokio::test]
async fn the_entries_map_back_by_position_in_both_releases() -> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(&server, version, answers()).await;

        let outcomes = client(&server, version)?.batch(&requests()).await?;
        assert_eq!(3, outcomes.len());

        let mut held = outcomes.into_iter();
        let first = held.next().ok_or("no first entry")??;
        let BatchOutcome::Lookup(LookupOutcome::Found { display, .. }) = first else {
            return Err(format!("the first entry is not a found code: {first:?}").into());
        };
        assert_eq!(fixtures::TERM_MEMBER_DISPLAY, display);

        let second = held.next().ok_or("no second entry")??;
        let BatchOutcome::Lookup(LookupOutcome::NotFound { outcome }) = second else {
            return Err(format!("the second entry is not a not-found: {second:?}").into());
        };
        assert!(outcome.has_tx_issue("invalid-code"));

        let third = held.next().ok_or("no third entry")??;
        let BatchOutcome::ValidateCode(ValidateOutcome::Valid { display, .. }) = third else {
            return Err(format!("the third entry is not a valid code: {third:?}").into());
        };
        assert_eq!(Some(fixtures::TERM_MEMBER_DISPLAY), display.as_deref());
    }
    Ok(())
}

#[tokio::test]
async fn the_bundle_carries_one_post_entry_per_request() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(&server, WireVersion::R4, answers()).await;
    client(&server, WireVersion::R4)?.batch(&requests()).await?;

    let body = request_body(&server, 0).await?;
    assert_eq!(
        Some(&serde_json::Value::from("batch")),
        body.get("type"),
        "a bundle of independent operations is a batch"
    );
    let entries = body
        .get("entry")
        .and_then(serde_json::Value::as_array)
        .ok_or("the bundle carries no entries")?;
    let urls: Vec<Option<&str>> = entries
        .iter()
        .map(|entry| {
            entry
                .get("request")
                .and_then(|request| request.get("url"))
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    assert_eq!(
        vec![
            Some(ferrobridge_term::LOOKUP),
            Some(ferrobridge_term::LOOKUP),
            Some(ferrobridge_term::VALIDATE_CODE),
        ],
        urls
    );
    for entry in entries {
        assert_eq!(
            Some("POST"),
            entry
                .get("request")
                .and_then(|request| request.get("method"))
                .and_then(serde_json::Value::as_str)
        );
        assert_eq!(
            Some("Parameters"),
            entry
                .get("resource")
                .and_then(|resource| resource.get("resourceType"))
                .and_then(serde_json::Value::as_str)
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_failed_entry_is_that_entrys_typed_error() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        terminology::batch_response(&[
            (
                "422 Unprocessable Entity",
                terminology::outcome_body(
                    "business-rule",
                    Some("vs-invalid"),
                    "the value set cannot be evaluated",
                ),
            ),
            (
                "200 OK",
                terminology::parameters_resource(&terminology::lookup_body(
                    "FerroBridgeSyntheticSource",
                    fixtures::TERM_MEMBER_DISPLAY,
                    None,
                    &[],
                    &[],
                )),
            ),
        ]),
    )
    .await;

    let sent = vec![
        Request::ValidateCode {
            value_set_url: fixtures::TERM_VALUE_SET_URL.to_owned(),
            system: fixtures::TERM_SOURCE_SYSTEM.to_owned(),
            code: fixtures::TERM_MEMBER_CODE.to_owned(),
        },
        Request::Lookup {
            system: fixtures::TERM_SOURCE_SYSTEM.to_owned(),
            code: fixtures::TERM_MEMBER_CODE.to_owned(),
            version: None,
        },
    ];
    let outcomes = client(&server, WireVersion::R4)?.batch(&sent).await?;
    let mut held = outcomes.into_iter();
    let error = held
        .next()
        .ok_or("no first entry")?
        .expect_err("the first entry was refused");
    let Error::Refused {
        operation,
        upstream,
        ..
    } = &error
    else {
        return Err(format!("expected a refusal, got {error:?}").into());
    };
    assert_eq!(ferrobridge_term::VALIDATE_CODE, *operation);
    assert_eq!(http::StatusCode::UNPROCESSABLE_ENTITY, upstream.status());
    assert!(
        held.next().ok_or("no second entry")?.is_ok(),
        "a refused entry takes only its own entry down"
    );
    Ok(())
}

#[tokio::test]
async fn a_short_answer_is_an_arity_error() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        terminology::batch_response(&[(
            "200 OK",
            terminology::parameters_resource(&terminology::lookup_body(
                "FerroBridgeSyntheticSource",
                fixtures::TERM_MEMBER_DISPLAY,
                None,
                &[],
                &[],
            )),
        )]),
    )
    .await;

    let error = client(&server, WireVersion::R4)?
        .batch(&requests())
        .await
        .expect_err("three requests answered by one entry cannot map back");
    let Error::BatchArity { sent, answered } = error else {
        return Err(format!("expected an arity error, got {error:?}").into());
    };
    assert_eq!((3, 1), (sent, answered));
    Ok(())
}

#[tokio::test]
async fn a_bundle_that_is_not_a_batch_response_is_a_decode_error() -> Result<(), Box<dyn StdError>>
{
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        ResponseTemplate::new(200)
            .set_body_string(r#"{"resourceType":"Bundle","type":"searchset"}"#),
    )
    .await;

    let error = client(&server, WireVersion::R4)?
        .batch(&requests())
        .await
        .expect_err("a searchset is not the answer to a batch");
    assert_eq!("body", error.kind());
    Ok(())
}
