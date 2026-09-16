// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The client against a real FHIR terminology server in a container.
//!
//! The server is started over the synthetic `CodeSystem`, `ValueSet` and
//! `ConceptMap` fixtures, and the three operations run against a server that
//! answers for itself. The test runs only when `FERROBRIDGE_E2E=1` admits the
//! container harness.

use crate::support::release_path;
use ferrobridge_term::client::Client;
use ferrobridge_term::config::{Config, WireVersion};
use ferrobridge_term::outcome::{LookupOutcome, Request, TranslateOutcome, ValidateOutcome};
use ferrobridge_testkit::containers;
use ferrobridge_testkit::fixtures;
use std::error::Error;

#[tokio::test]
async fn the_client_resolves_translates_and_validates_against_a_real_server()
-> Result<(), Box<dyn Error>> {
    if !containers::e2e_enabled() {
        return Ok(());
    }
    let server = containers::terminology().await?;
    for version in crate::support::RELEASES {
        let base = server.base_url(&release_path(version));
        let client = Client::new(Config::new(base.parse()?, version))?;
        resolve_a_display(&client, version).await?;
        refuse_an_unknown_code(&client, version).await?;
        translate_a_mapped_code(&client, version).await?;
        refuse_a_wider_match(&client, version).await?;
        validate_a_member_and_a_non_member(&client, version).await?;
        answer_a_batch_by_position(&client, version).await?;
    }
    Ok(())
}

/// Reads the display of the synthetic member concept.
async fn resolve_a_display(client: &Client, version: WireVersion) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .lookup(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_MEMBER_CODE,
            None,
        )
        .await?;
    let LookupOutcome::Found {
        display,
        designations,
        ..
    } = outcome
    else {
        return Err(format!(
            "{}: expected a found code, got {outcome:?}",
            version.as_str()
        )
        .into());
    };
    assert_eq!(
        fixtures::TERM_MEMBER_DISPLAY,
        display,
        "{}: the server resolved a different display",
        version.as_str()
    );
    assert!(
        designations
            .iter()
            .any(|designation| designation.language.as_deref() == Some("nl")),
        "{}: the Dutch designation of the fixture was not served",
        version.as_str()
    );
    Ok(())
}

/// Reads a code no fixture defines.
async fn refuse_an_unknown_code(
    client: &Client,
    version: WireVersion,
) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .lookup(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_UNKNOWN_CODE,
            None,
        )
        .await?;
    let LookupOutcome::NotFound { outcome } = outcome else {
        return Err(format!(
            "{}: expected a not-found, got {outcome:?}",
            version.as_str()
        )
        .into());
    };
    assert!(
        outcome.tx_issues().count() > 0,
        "{}: the server sent no tx-issue-type coding: {}",
        version.as_str(),
        outcome.body()
    );
    Ok(())
}

/// Translates the concept the map states as `equivalent`.
async fn translate_a_mapped_code(
    client: &Client,
    version: WireVersion,
) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .translate(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_MEMBER_CODE,
            fixtures::TERM_TARGET_SYSTEM,
            Some(fixtures::TERM_CONCEPT_MAP_URL),
        )
        .await?;
    let accepted: Vec<Option<String>> = outcome
        .accepted()
        .map(|found| {
            found
                .concept
                .as_ref()
                .and_then(|concept| concept.code.clone())
        })
        .collect();
    assert_eq!(
        vec![Some(fixtures::TERM_MEMBER_TARGET_CODE.to_owned())],
        accepted,
        "{}: the equivalent match did not survive: {outcome:?}",
        version.as_str()
    );
    Ok(())
}

/// Translates the concept the map states as `wider`.
async fn refuse_a_wider_match(client: &Client, version: WireVersion) -> Result<(), Box<dyn Error>> {
    let outcome = client
        .translate(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_NON_MEMBER_CODE,
            fixtures::TERM_TARGET_SYSTEM,
            Some(fixtures::TERM_CONCEPT_MAP_URL),
        )
        .await?;
    if let TranslateOutcome::Translated { matches, .. } = &outcome {
        assert!(
            !matches.is_empty(),
            "{}: the server returned no match at all",
            version.as_str()
        );
    }
    assert_eq!(
        0,
        outcome.accepted().count(),
        "{}: a wider match was accepted as a translation: {outcome:?}",
        version.as_str()
    );
    Ok(())
}

/// Validates the member and the non-member of the synthetic value set.
async fn validate_a_member_and_a_non_member(
    client: &Client,
    version: WireVersion,
) -> Result<(), Box<dyn Error>> {
    let valid = client
        .validate_code(
            fixtures::TERM_VALUE_SET_URL,
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_MEMBER_CODE,
        )
        .await?;
    let ValidateOutcome::Valid { display, .. } = &valid else {
        return Err(format!("{}: expected a valid code, got {valid:?}", version.as_str()).into());
    };
    assert_eq!(
        Some(fixtures::TERM_MEMBER_DISPLAY),
        display.as_deref(),
        "{}: the valid answer carried a different display",
        version.as_str()
    );

    let invalid = client
        .validate_code(
            fixtures::TERM_VALUE_SET_URL,
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_NON_MEMBER_CODE,
        )
        .await?;
    assert!(
        !invalid.is_valid(),
        "{}: a non-member was accepted: {invalid:?}",
        version.as_str()
    );
    Ok(())
}

/// Sends the three operations as one batch and reads them back by position.
async fn answer_a_batch_by_position(
    client: &Client,
    version: WireVersion,
) -> Result<(), Box<dyn Error>> {
    let requests = vec![
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
    ];
    let outcomes = client.batch(&requests).await?;
    assert_eq!(
        2,
        outcomes.len(),
        "{}: the batch did not answer one entry per request",
        version.as_str()
    );
    let mut held = outcomes.into_iter();
    let first = held.next().ok_or("no first entry")??;
    assert!(
        format!("{first:?}").contains(fixtures::TERM_MEMBER_DISPLAY),
        "{}: the first entry is not the display: {first:?}",
        version.as_str()
    );
    let second = held.next().ok_or("no second entry")??;
    assert!(
        matches!(
            second,
            ferrobridge_term::outcome::BatchOutcome::Lookup(LookupOutcome::NotFound { .. })
        ),
        "{}: the second entry is not a not-found: {second:?}",
        version.as_str()
    );
    Ok(())
}
