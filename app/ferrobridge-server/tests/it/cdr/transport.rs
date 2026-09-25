// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The transport contract of the bridge's CDR client: retry, timeout,
//! credentials and correlation as `[cdr]` configures the generated client, and
//! the upstream answer kept beside every refusal.

use super::support;
use ferrobridge_server::cdr::CdrClient;
use ferrobridge_server::cdr::Prefer;
use ferrobridge_server::cdr::config::CdrConfig;
use ferrobridge_server::cdr::error::CdrError;
use ferrobridge_server::cdr::ids::{EhrId, RequestId};
use openehr_its::rest::client::{ClientError, Credentials, RetryPolicy, TransportError};
use openehr_its::rest::generated::ehr::client::EhrGetByIdOutcome;
use secrecy::SecretString;
use std::error::Error;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The EHR every case in this module reads.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// Returns the generated-client refusal and the kept answer inside `error`.
fn refusal(
    error: CdrError,
) -> Result<
    (
        ClientError,
        Option<ferrobridge_server::cdr::error::Upstream>,
    ),
    Box<dyn Error>,
> {
    match error {
        CdrError::Client { source, upstream } => Ok((*source, upstream.map(|upstream| *upstream))),
        other => Err(format!("expected a refusal of the generated client, got {other:?}").into()),
    }
}

#[tokio::test]
async fn a_get_retries_a_503_and_succeeds_on_the_second_attempt() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(support::EHR_JSON))
        .expect(1)
        .mount(&server)
        .await;

    let answered = support::client(&server)?.ehr(&EhrId::new(EHR)?).await?;
    assert!(matches!(answered.outcome, EhrGetByIdOutcome::Ok { .. }));
    assert_eq!(http::StatusCode::OK, answered.upstream.status());
    assert_eq!(
        2,
        server
            .received_requests()
            .await
            .ok_or("the mock server is recording")?
            .len()
    );
    Ok(())
}

#[tokio::test]
async fn a_post_is_not_retried_after_a_503() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ehr"))
        .respond_with(ResponseTemplate::new(503).set_body_string("the service is restarting"))
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .create_ehr(None, Prefer::Minimal)
        .await
        .expect_err("a 503 is an error, never an outcome");
    let (source, upstream) = refusal(error)?;
    assert!(
        matches!(source, ClientError::ServiceFailure { status, .. } if status == http::StatusCode::SERVICE_UNAVAILABLE),
        "{source:?}"
    );
    let upstream = upstream.ok_or("the refusal keeps the answer")?;
    assert_eq!(http::StatusCode::SERVICE_UNAVAILABLE, upstream.status());
    assert_eq!("the service is restarting", upstream.body());
    assert_eq!(
        1,
        server
            .received_requests()
            .await
            .ok_or("the mock server is recording")?
            .len()
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_connection_is_a_typed_transport_error() -> Result<(), Box<dyn Error>> {
    // NOTE: no specification governs this: our own design. Port 1 is reserved
    // and unbindable, so the connection is refused rather than answered.
    let client = CdrClient::new(
        &CdrConfig::new("http://127.0.0.1:1/v1".parse()?)
            .with_timeout(Duration::from_secs(2))
            .with_retry(RetryPolicy {
                max_attempts: 2,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(2),
            }),
    )?;
    let error = client
        .ehr(&EhrId::new(EHR)?)
        .await
        .expect_err("a closed port refuses the connection");
    let (source, upstream) = refusal(error)?;
    match source {
        ClientError::Transport { source, .. } => {
            assert!(
                source.source().is_some(),
                "the transport error keeps its cause"
            );
        }
        other => return Err(format!("expected a transport error, got {other:?}").into()),
    }
    assert!(upstream.is_none(), "no answer was read");
    Ok(())
}

#[tokio::test]
async fn a_slow_answer_past_the_timeout_is_a_typed_timeout() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(400))
                .set_body_string(support::EHR_JSON),
        )
        .mount(&server)
        .await;

    let client = CdrClient::new(
        &CdrConfig::new(format!("{}/v1", server.uri()).parse()?)
            .with_timeout(Duration::from_millis(50))
            .with_retry(RetryPolicy {
                max_attempts: 1,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(2),
            }),
    )?;
    let error = client
        .ehr(&EhrId::new(EHR)?)
        .await
        .expect_err("the answer arrives past the timeout");
    let (source, _upstream) = refusal(error)?;
    assert!(
        matches!(
            source,
            ClientError::Transport {
                source: TransportError::Timeout { .. },
                ..
            }
        ),
        "{source:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_bearer_token_travels_and_never_reaches_the_debug_rendering() -> Result<(), Box<dyn Error>>
{
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(support::EHR_JSON))
        .mount(&server)
        .await;

    let client = CdrClient::new(
        &CdrConfig::new(format!("{}/v1", server.uri()).parse()?)
            .with_credentials(Credentials::Bearer(SecretString::from("synthetic-token"))),
    )?;
    let client = client.with_request_id(RequestId::new("corr-42")?);
    let _answered = client.ehr(&EhrId::new(EHR)?).await?;

    assert_eq!(
        vec!["Bearer synthetic-token".to_owned()],
        support::request_header(&server, 0, "authorization").await?
    );
    assert_eq!(
        vec!["corr-42".to_owned()],
        support::request_header(&server, 0, "x-request-id").await?
    );
    assert!(!format!("{client:?}").contains("synthetic-token"));
    Ok(())
}

#[tokio::test]
async fn basic_credentials_travel_as_rfc_7617() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(support::EHR_JSON))
        .mount(&server)
        .await;

    let client = CdrClient::new(
        &CdrConfig::new(format!("{}/v1", server.uri()).parse()?).with_credentials(
            Credentials::Basic {
                user: String::from("bridge"),
                password: SecretString::from("synthetic-password"),
            },
        ),
    )?;
    let _answered = client.ehr(&EhrId::new(EHR)?).await?;

    // NOTE: RFC 7617 §2, the credentials are base64 of `user:password`;
    // `YnJpZGdlOnN5bnRoZXRpYy1wYXNzd29yZA==` is `bridge:synthetic-password`.
    assert_eq!(
        vec!["Basic YnJpZGdlOnN5bnRoZXRpYy1wYXNzd29yZA==".to_owned()],
        support::request_header(&server, 0, "authorization").await?
    );
    assert!(!format!("{client:?}").contains("synthetic-password"));
    Ok(())
}

#[tokio::test]
async fn a_401_keeps_the_challenge() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Bearer realm=\"cdr\""),
        )
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .ehr(&EhrId::new(EHR)?)
        .await
        .expect_err("a 401 is an error, never an outcome");
    let (source, upstream) = refusal(error)?;
    match source {
        ClientError::Unauthorized { challenge, .. } => {
            assert_eq!(Some("Bearer realm=\"cdr\"".to_owned()), challenge);
        }
        other => return Err(format!("expected an unauthorized error, got {other:?}").into()),
    }
    assert_eq!(
        Some(http::StatusCode::UNAUTHORIZED),
        upstream.map(|upstream| upstream.status())
    );
    Ok(())
}

#[tokio::test]
async fn an_undocumented_status_is_an_error_carrying_the_body() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}")))
        .respond_with(ResponseTemplate::new(418).set_body_string("no tea here"))
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .ehr(&EhrId::new(EHR)?)
        .await
        .expect_err("418 is not a documented answer of this operation");
    let (source, upstream) = refusal(error)?;
    assert!(
        matches!(source, ClientError::UndocumentedStatus { status, .. } if status == http::StatusCode::IM_A_TEAPOT),
        "{source:?}"
    );
    let upstream = upstream.ok_or("the refusal keeps the answer")?;
    assert_eq!(http::StatusCode::IM_A_TEAPOT, upstream.status());
    assert_eq!("no tea here", upstream.body());
    Ok(())
}

#[tokio::test]
async fn a_reachability_probe_reports_every_status_the_service_answers()
-> Result<(), Box<dyn Error>> {
    for answered in [200_u16, 401, 404, 503] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(answered))
            .expect(1)
            .mount(&server)
            .await;

        let status = support::client(&server)?.reachability().await?;
        assert_eq!(answered, status.as_u16(), "the probe reports what it saw");
    }
    Ok(())
}

#[tokio::test]
async fn a_reachability_probe_that_never_connects_is_a_transport_error()
-> Result<(), Box<dyn Error>> {
    let config =
        CdrConfig::new("http://127.0.0.1:1/v1".parse()?).with_timeout(Duration::from_secs(2));

    let outcome = CdrClient::new(&config)?.reachability().await;
    assert!(
        matches!(
            &outcome,
            Err(CdrError::Client { source, .. }) if matches!(**source, ClientError::Transport { .. })
        ),
        "an unreachable service is a typed transport error: {outcome:?}"
    );
    Ok(())
}
