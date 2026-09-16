// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ValueSet/$validate-code`
//! (<https://hl7.org/fhir/R4/valueset-operation-validate-code.html>).

use crate::support::{RELEASES, client, parameter, release_path, request_body};
use ferrobridge_term::config::WireVersion;
use ferrobridge_term::error::Error;
use ferrobridge_term::outcome::ValidateOutcome;
use ferrobridge_testkit::fixtures;
use ferrobridge_testkit::stubs::terminology;
use std::error::Error as StdError;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The route the operation is invoked on, under the release prefix.
fn route(version: WireVersion) -> String {
    format!("/{}/ValueSet/$validate-code", release_path(version))
}

/// Mounts `response` as the one answer of the `$validate-code` route.
async fn mount(server: &MockServer, version: WireVersion, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(route(version)))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Sends the fixture validation of `code` to `server`.
async fn validate(
    server: &MockServer,
    version: WireVersion,
    code: &str,
) -> Result<ValidateOutcome, Error> {
    client(server, version)
        .expect("a buildable client")
        .validate_code(
            fixtures::TERM_VALUE_SET_URL,
            fixtures::TERM_SOURCE_SYSTEM,
            code,
        )
        .await
}

#[tokio::test]
async fn a_member_is_valid_and_carries_its_display_in_both_releases()
-> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::validate_code(true, Some(fixtures::TERM_MEMBER_DISPLAY), None),
        )
        .await;

        let outcome = validate(&server, version, fixtures::TERM_MEMBER_CODE).await?;
        let ValidateOutcome::Valid { display, .. } = &outcome else {
            return Err(format!("expected a valid code, got {outcome:?}").into());
        };
        assert!(outcome.is_valid());
        assert_eq!(Some(fixtures::TERM_MEMBER_DISPLAY), display.as_deref());
    }
    Ok(())
}

#[tokio::test]
async fn a_non_member_is_invalid_and_surfaces_its_issues_outcome() -> Result<(), Box<dyn StdError>>
{
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::validate_code_invalid(
                "the provided code was not found in the value set",
                "not-in-vs",
            ),
        )
        .await;

        let outcome = validate(&server, version, fixtures::TERM_NON_MEMBER_CODE).await?;
        let ValidateOutcome::Invalid {
            message,
            outcome: issues,
        } = &outcome
        else {
            return Err(format!("expected an invalid code, got {outcome:?}").into());
        };
        assert!(!outcome.is_valid());
        assert_eq!(
            Some("the provided code was not found in the value set"),
            message.as_deref()
        );
        let issues = issues.as_ref().ok_or("the issues outcome was dropped")?;
        assert!(
            issues.has_tx_issue("not-in-vs"),
            "the tx-issue-type coding of the issues outcome was dropped"
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
            terminology::validate_code(true, Some(fixtures::TERM_MEMBER_DISPLAY), None),
        )
        .await;
        validate(&server, version, fixtures::TERM_MEMBER_CODE).await?;

        let body = request_body(&server, 0).await?;
        assert_eq!(
            Some((
                "valueUri".to_owned(),
                serde_json::Value::from(fixtures::TERM_VALUE_SET_URL)
            )),
            parameter(&body, "url")
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
    }
    Ok(())
}

#[tokio::test]
async fn an_unknown_value_set_is_a_typed_error_and_never_an_invalid_code()
-> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        terminology::outcome(
            404,
            "not-found",
            Some("not-found"),
            "no ValueSet with that canonical",
        ),
    )
    .await;

    let error = validate(&server, WireVersion::R4, fixtures::TERM_MEMBER_CODE)
        .await
        .expect_err("an unknown value set is a refusal");
    let Error::Refused {
        operation,
        upstream,
        ..
    } = &error
    else {
        return Err(format!("expected a refusal, got {error:?}").into());
    };
    assert_eq!(ferrobridge_term::VALIDATE_CODE, *operation);
    assert!(upstream.has_tx_issue("not-found"));
    Ok(())
}
