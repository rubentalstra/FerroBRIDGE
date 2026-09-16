// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The transport rules: the retry budget, the timeout, the credentials and
//! the `401` challenge.
//!
//! The three operations declare `affectsState: false`
//! (`OperationDefinition-CodeSystem-lookup`, `-ConceptMap-translate` and
//! `-ValueSet-validate-code`, R4B), so the `POST` form is retried like a read.

use crate::support::{client, config, release_path};
use ferrobridge_term::client::Client;
use ferrobridge_term::config::{Config, Credentials, RetryPolicy, WireVersion};
use ferrobridge_term::error::Error;
use ferrobridge_term::outcome::LookupOutcome;
use ferrobridge_testkit::fixtures;
use ferrobridge_testkit::stubs::terminology;
use secrecy::SecretString;
use std::error::Error as StdError;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The `$lookup` route under the R4 prefix.
fn route() -> String {
    format!("/{}/CodeSystem/$lookup", release_path(WireVersion::R4))
}

/// Sends the fixture lookup to `client`.
async fn lookup(client: &Client) -> Result<LookupOutcome, Error> {
    client
        .lookup(
            fixtures::TERM_SOURCE_SYSTEM,
            fixtures::TERM_MEMBER_CODE,
            None,
        )
        .await
}

#[tokio::test]
async fn a_503_is_retried_and_the_second_attempt_answers() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route()))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(route()))
        .respond_with(terminology::lookup(
            "FerroBridgeSyntheticSource",
            fixtures::TERM_MEMBER_DISPLAY,
            None,
        ))
        .mount(&server)
        .await;

    let outcome = lookup(&client(&server, WireVersion::R4)?).await?;
    assert!(
        matches!(outcome, LookupOutcome::Found { .. }),
        "the retry did not reach the answer: {outcome:?}"
    );
    let requests = server
        .received_requests()
        .await
        .ok_or("the mock server is not recording requests")?;
    assert_eq!(
        2,
        requests.len(),
        "the request was not retried exactly once"
    );
    Ok(())
}

#[tokio::test]
async fn an_exhausted_budget_answers_the_upstream_status() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route()))
        .respond_with(terminology::outcome(
            500,
            "exception",
            None,
            "the provider failed",
        ))
        .mount(&server)
        .await;

    let error = lookup(&client(&server, WireVersion::R4)?)
        .await
        .expect_err("every attempt answered 500");
    let Error::ServerFailure { upstream, .. } = &error else {
        return Err(format!("expected a server failure, got {error:?}").into());
    };
    assert_eq!(http::StatusCode::INTERNAL_SERVER_ERROR, upstream.status());
    assert!(error.is_retryable());
    let requests = server
        .received_requests()
        .await
        .ok_or("the mock server is not recording requests")?;
    assert_eq!(
        3,
        requests.len(),
        "the budget of three attempts was not used"
    );
    Ok(())
}

#[tokio::test]
async fn a_timeout_is_a_typed_error_and_never_an_empty_answer() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route()))
        .respond_with(
            terminology::lookup("FerroBridgeSyntheticSource", "Alpha finding", None)
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&server)
        .await;

    let client = Client::new(
        config(&server, WireVersion::R4)?
            .with_timeout(Duration::from_millis(50))
            .with_retry(RetryPolicy {
                max_attempts: 1,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
            }),
    )?;
    let error = lookup(&client).await.expect_err("the server was too slow");
    assert_eq!("timeout", error.kind());
    assert!(error.is_retryable());
    Ok(())
}

#[tokio::test]
async fn a_401_keeps_the_www_authenticate_challenge() -> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route()))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("WWW-Authenticate", "Bearer realm=\"terminology\""),
        )
        .mount(&server)
        .await;

    let client = Client::new(
        config(&server, WireVersion::R4)?
            .with_credentials(Credentials::Bearer(SecretString::from("s3cr3t-token"))),
    )?;
    let error = lookup(&client)
        .await
        .expect_err("the credentials were refused");
    let Error::Unauthorized { challenge, .. } = &error else {
        return Err(format!("expected an unauthorized error, got {error:?}").into());
    };
    assert_eq!(Some("Bearer realm=\"terminology\""), challenge.as_deref());
    assert!(
        !error.is_retryable(),
        "a refused credential is not transient"
    );
    Ok(())
}

#[tokio::test]
async fn the_bearer_token_travels_and_never_reaches_a_debug_rendering()
-> Result<(), Box<dyn StdError>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route()))
        .respond_with(terminology::lookup(
            "FerroBridgeSyntheticSource",
            fixtures::TERM_MEMBER_DISPLAY,
            None,
        ))
        .mount(&server)
        .await;

    let client = Client::new(
        config(&server, WireVersion::R4)?
            .with_credentials(Credentials::Bearer(SecretString::from("s3cr3t-token"))),
    )?;
    lookup(&client).await?;

    assert_eq!(
        vec!["Bearer s3cr3t-token".to_owned()],
        crate::support::request_header(&server, 0, "authorization").await?
    );
    assert!(
        !format!("{client:?}").contains("s3cr3t-token"),
        "the token is in the client's Debug rendering"
    );
    assert!(
        !format!("{:?}", client.config()).contains("s3cr3t-token"),
        "the token is in the configuration's Debug rendering"
    );
    Ok(())
}

#[tokio::test]
async fn an_unreachable_server_is_a_transport_error() -> Result<(), Box<dyn StdError>> {
    // Port 1 on the loopback interface refuses the connection at once, so the
    // failure is the transport's rather than a slow address.
    let config =
        Config::new("http://127.0.0.1:1/r4".parse()?, WireVersion::R4).with_retry(RetryPolicy {
            max_attempts: 1,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        });

    let error = lookup(&Client::new(config)?)
        .await
        .expect_err("nothing listens on that port");
    assert_eq!(
        "transport",
        error.kind(),
        "a refused connection is not a transport failure"
    );
    assert!(error.is_retryable(), "a refused connection is transient");
    Ok(())
}

#[tokio::test]
async fn a_reachability_probe_reads_the_capability_statement_route() -> Result<(), Box<dyn StdError>>
{
    for answered in [200_u16, 401, 404, 503] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/{}/metadata", release_path(WireVersion::R4))))
            .respond_with(ResponseTemplate::new(answered))
            .expect(1)
            .mount(&server)
            .await;

        let status = client(&server, WireVersion::R4)?.reachability().await?;
        assert_eq!(answered, status.as_u16(), "the probe reports what it saw");
    }
    Ok(())
}

#[tokio::test]
async fn a_reachability_probe_that_never_connects_is_a_transport_error()
-> Result<(), Box<dyn StdError>> {
    let config = Config::new("http://127.0.0.1:1/r4".parse()?, WireVersion::R4);

    let outcome = Client::new(config)?.reachability().await;
    assert!(
        matches!(outcome, Err(Error::Transport { .. })),
        "an unreachable server is a typed transport error"
    );
    Ok(())
}
