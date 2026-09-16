// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ConceptMap/$translate`
//! (<https://hl7.org/fhir/R4/conceptmap-operation-translate.html>).

use crate::support::{RELEASES, client, parameter, release_path, request_body};
use ferrobridge_term::concept::Equivalence;
use ferrobridge_term::config::WireVersion;
use ferrobridge_term::error::Error;
use ferrobridge_term::outcome::TranslateOutcome;
use ferrobridge_testkit::fixtures;
use ferrobridge_testkit::stubs::terminology;
use std::error::Error as StdError;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The route the operation is invoked on, under the release prefix.
fn route(version: WireVersion) -> String {
    format!("/{}/ConceptMap/$translate", release_path(version))
}

/// Mounts `response` as the one answer of the `$translate` route.
async fn mount(server: &MockServer, version: WireVersion, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(route(version)))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Sends the fixture translation of `code` to `server`.
async fn translate(
    server: &MockServer,
    version: WireVersion,
    code: &str,
) -> Result<TranslateOutcome, Error> {
    client(server, version)
        .expect("a buildable client")
        .translate(
            fixtures::TERM_SOURCE_SYSTEM,
            code,
            fixtures::TERM_TARGET_SYSTEM,
            Some(fixtures::TERM_CONCEPT_MAP_URL),
        )
        .await
}

#[tokio::test]
async fn an_equivalent_match_is_a_translation_in_both_releases() -> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::translate(
                "equivalent",
                fixtures::TERM_TARGET_SYSTEM,
                fixtures::TERM_MEMBER_TARGET_CODE,
                "Target alpha",
            ),
        )
        .await;

        let outcome = translate(&server, version, fixtures::TERM_MEMBER_CODE).await?;
        let TranslateOutcome::Translated { matches, .. } = &outcome else {
            return Err(format!("expected a translation, got {outcome:?}").into());
        };
        assert_eq!(1, matches.len());
        let accepted: Vec<Option<&str>> = outcome
            .accepted()
            .map(|found| {
                found
                    .concept
                    .as_ref()
                    .and_then(|concept| concept.code.as_deref())
            })
            .collect();
        assert_eq!(vec![Some(fixtures::TERM_MEMBER_TARGET_CODE)], accepted);
    }
    Ok(())
}

#[tokio::test]
async fn a_wider_or_inexact_match_is_returned_and_never_accepted() -> Result<(), Box<dyn StdError>>
{
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        terminology::translate_matches(&[
            (
                "wider",
                fixtures::TERM_TARGET_SYSTEM,
                "B1",
                "Target beta group",
            ),
            (
                "inexact",
                fixtures::TERM_TARGET_SYSTEM,
                "B2",
                "Target beta neighbour",
            ),
        ]),
    )
    .await;

    let outcome = translate(&server, WireVersion::R4, fixtures::TERM_NON_MEMBER_CODE).await?;
    let TranslateOutcome::Translated { matches, .. } = &outcome else {
        return Err(format!("expected the matches, got {outcome:?}").into());
    };
    assert_eq!(
        2,
        matches.len(),
        "the caller checks each match, so none is dropped"
    );
    assert_eq!(
        vec![Some("wider"), Some("inexact")],
        matches
            .iter()
            .map(|found| found.equivalence.as_ref().map(Equivalence::as_str))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        0,
        outcome.accepted().count(),
        "a wider or inexact match is not a translation"
    );
    Ok(())
}

#[tokio::test]
async fn a_false_result_is_a_no_match_outcome() -> Result<(), Box<dyn StdError>> {
    for version in RELEASES {
        let server = MockServer::start().await;
        mount(
            &server,
            version,
            terminology::translate_no_match("no map for this code"),
        )
        .await;

        let outcome = translate(&server, version, fixtures::TERM_UNKNOWN_CODE).await?;
        let TranslateOutcome::NoMatch { message } = &outcome else {
            return Err(format!("expected no match, got {outcome:?}").into());
        };
        assert_eq!(Some("no map for this code"), message.as_deref());
        assert_eq!(0, outcome.accepted().count());
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
            terminology::translate(
                "equivalent",
                fixtures::TERM_TARGET_SYSTEM,
                fixtures::TERM_MEMBER_TARGET_CODE,
                "Target alpha",
            ),
        )
        .await;
        translate(&server, version, fixtures::TERM_MEMBER_CODE).await?;

        let body = request_body(&server, 0).await?;
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
            Some((
                "valueUri".to_owned(),
                serde_json::Value::from(fixtures::TERM_TARGET_SYSTEM)
            )),
            parameter(&body, "targetsystem"),
            "both releases spell the target system parameter `targetsystem`"
        );
        assert_eq!(
            Some((
                "valueUri".to_owned(),
                serde_json::Value::from(fixtures::TERM_CONCEPT_MAP_URL)
            )),
            parameter(&body, "url")
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_unknown_concept_map_is_a_typed_error_and_never_a_missing_translation()
-> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    mount(
        &server,
        WireVersion::R4,
        terminology::outcome(
            404,
            "not-found",
            Some("not-found"),
            "no ConceptMap with that canonical",
        ),
    )
    .await;

    let error = translate(&server, WireVersion::R4, fixtures::TERM_MEMBER_CODE)
        .await
        .expect_err("an unknown concept map fails closed");
    let Error::Refused {
        operation,
        upstream,
        ..
    } = &error
    else {
        return Err(format!("expected a refusal, got {error:?}").into());
    };
    assert_eq!(ferrobridge_term::TRANSLATE, *operation);
    assert!(upstream.has_tx_issue("not-found"));
    Ok(())
}
