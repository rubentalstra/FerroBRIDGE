// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! `CodeSystem/$lookup`
//! (<https://hl7.org/fhir/R4/codesystem-operation-lookup.html>).

use crate::support::{RELEASES, client, parameter, release_path, request_body, request_header};
use ferrobridge_term::config::WireVersion;
use ferrobridge_term::error::Error;
use ferrobridge_term::outcome::LookupOutcome;
use ferrobridge_testkit::fixtures;
use ferrobridge_testkit::stubs::terminology;
use std::error::Error as StdError;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The route the operation is invoked on, under the release prefix.
fn route(version: WireVersion) -> String {
    format!("/{}/CodeSystem/$lookup", release_path(version))
}

/// Mounts `response` as the one answer of the `$lookup` route.
async fn mount(server: &MockServer, version: WireVersion, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(route(version)))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_found_code_answers_its_display_in_both_releases() -> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::lookup_detailed(
                "FerroBridgeSyntheticSource",
                fixtures::TERM_MEMBER_DISPLAY,
                Some("1.0.0"),
                &[("nl", "Alfa bevinding")],
                &[("inactive", "valueBoolean", serde_json::Value::Bool(false))],
            ),
        )
        .await;

        let outcome = client(&server, version)?
            .lookup(
                fixtures::TERM_SOURCE_SYSTEM,
                fixtures::TERM_MEMBER_CODE,
                Some("1.0.0"),
            )
            .await?;
        let LookupOutcome::Found {
            name,
            version: code_system_version,
            display,
            designations,
            properties,
        } = outcome
        else {
            return Err(format!(
                "expected a found code in {}, got {outcome:?}",
                version.as_str()
            )
            .into());
        };
        assert_eq!("FerroBridgeSyntheticSource", name);
        assert_eq!(Some("1.0.0".to_owned()), code_system_version);
        assert_eq!(fixtures::TERM_MEMBER_DISPLAY, display);
        assert_eq!(1, designations.len(), "the designation was dropped");
        assert_eq!(
            Some("nl"),
            designations
                .first()
                .and_then(|designation| designation.language.as_deref())
        );
        let property = properties.first().ok_or("the property was dropped")?;
        assert_eq!("inactive", property.code);
        assert_eq!(
            Some("valueBoolean"),
            property.value.as_ref().map(|value| value.member.as_str()),
            "the choice member the value arrived in was lost"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_request_carries_the_declared_parameter_names() -> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::lookup("FerroBridgeSyntheticSource", "Alpha finding", None),
        )
        .await;
        client(&server, version)?
            .lookup(
                fixtures::TERM_SOURCE_SYSTEM,
                fixtures::TERM_MEMBER_CODE,
                Some("1.0.0"),
            )
            .await?;

        let body = request_body(&server, 0).await?;
        assert_eq!(
            Some(serde_json::Value::from("Parameters")),
            body.get("resourceType").cloned()
        );
        assert_eq!(
            Some((
                "valueUri".to_owned(),
                serde_json::Value::from(fixtures::TERM_SOURCE_SYSTEM)
            )),
            parameter(&body, "system")
        );
        assert_eq!(
            Some((
                "valueCode".to_owned(),
                serde_json::Value::from(fixtures::TERM_MEMBER_CODE)
            )),
            parameter(&body, "code")
        );
        assert_eq!(
            Some(("valueString".to_owned(), serde_json::Value::from("1.0.0"))),
            parameter(&body, "version")
        );
        assert_eq!(
            vec!["application/fhir+json".to_owned()],
            request_header(&server, 0, "accept").await?
        );
        assert_eq!(
            vec!["application/fhir+json".to_owned()],
            request_header(&server, 0, "content-type").await?
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_unknown_code_is_a_not_found_outcome_carrying_its_tx_issue()
-> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::outcome(
                400,
                "code-invalid",
                Some("invalid-code"),
                "the code is not in the code system",
            ),
        )
        .await;

        let outcome = client(&server, version)?
            .lookup(
                fixtures::TERM_SOURCE_SYSTEM,
                fixtures::TERM_UNKNOWN_CODE,
                None,
            )
            .await?;
        let LookupOutcome::NotFound { outcome } = outcome else {
            return Err(format!("expected a not-found outcome, got {outcome:?}").into());
        };
        assert_eq!(http::StatusCode::BAD_REQUEST, outcome.status());
        assert!(
            outcome.has_tx_issue("invalid-code"),
            "the tx-issue-type coding was dropped"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_bare_404_with_no_outcome_is_a_not_found_outcome() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        ResponseTemplate::new(404).set_body_string("not found"),
    )
    .await;

    let outcome = client(&server, WireVersion::R4)?
        .lookup(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_UNKNOWN_CODE,
            None,
        )
        .await?;
    let LookupOutcome::NotFound { outcome } = outcome else {
        return Err(format!("expected a not-found outcome, got {outcome:?}").into());
    };
    assert_eq!("not found", outcome.body());
    Ok(())
}

#[tokio::test]
async fn another_refusal_is_a_typed_error_that_keeps_the_body() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        terminology::outcome(
            422,
            "business-rule",
            Some("vs-invalid"),
            "the code system cannot be evaluated",
        ),
    )
    .await;

    let error = client(&server, WireVersion::R4)?
        .lookup(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_MEMBER_CODE,
            None,
        )
        .await
        .expect_err("a 422 is a refusal, not an outcome");
    let Error::Refused {
        operation,
        upstream,
        ..
    } = &error
    else {
        return Err(format!("expected a refusal, got {error:?}").into());
    };
    assert_eq!(ferrobridge_term::LOOKUP, *operation);
    assert_eq!(http::StatusCode::UNPROCESSABLE_ENTITY, upstream.status());
    assert!(upstream.has_tx_issue("vs-invalid"));
    assert!(!error.is_retryable());
    Ok(())
}

#[tokio::test]
async fn an_answer_that_is_not_parameters_is_a_decode_error() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        ResponseTemplate::new(200).set_body_string("{\"resourceType\":\"Bundle\"}"),
    )
    .await;

    let error = client(&server, WireVersion::R4)?
        .lookup(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_MEMBER_CODE,
            None,
        )
        .await
        .expect_err("a Bundle is not the operation's answer");
    assert_eq!("body", error.kind());
    Ok(())
}
