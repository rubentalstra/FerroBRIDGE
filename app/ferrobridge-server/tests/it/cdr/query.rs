// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL contract over the generated client: one execution, and the
//! bridge's paged read consumed as a stream.

use super::support;
use ferrobridge_server::cdr::query::{PageSize, QueryPageError};
use futures_util::StreamExt;
use openehr_its::rest::generated::query::AdhocQueryExecute;
use openehr_its::rest::generated::query::client::QueryExecuteAdhocQueryBodyOutcome;
use std::error::Error;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A synthetic population query.
const AQL: &str = "SELECT c/uid/value AS cid FROM EHR e CONTAINS COMPOSITION c";

/// Returns the request body for the synthetic query.
fn request() -> AdhocQueryExecute {
    AdhocQueryExecute {
        q: AQL.to_owned(),
        offset: None,
        fetch: None,
        query_parameters: None,
    }
}

#[tokio::test]
async fn query_aql_reads_the_result_set() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query/aql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"columns":[{"name":"cid","path":"/uid/value"}],"rows":[["8849182c::system::1"]]}"#,
        ))
        .mount(&server)
        .await;

    let answered = support::client(&server)?.query_aql(&request()).await?;
    match answered.outcome {
        QueryExecuteAdhocQueryBodyOutcome::Ok { body, .. } => {
            assert_eq!(1, body.rows.len());
            assert_eq!(Some(1), body.columns.as_ref().map(Vec::len));
        }
        other => return Err(format!("expected a result set, got {other:?}").into()),
    }
    assert_eq!(
        vec!["return=representation".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn query_aql_reports_the_400() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query/aql"))
        .respond_with(
            ResponseTemplate::new(400).set_body_string(
                r#"{"message":"the AQL does not parse","code":90001,"errors":[]}"#,
            ),
        )
        .mount(&server)
        .await;

    let answered = support::client(&server)?.query_aql(&request()).await?;
    assert!(matches!(
        answered.outcome,
        QueryExecuteAdhocQueryBodyOutcome::BadRequest
    ));
    assert_eq!(http::StatusCode::BAD_REQUEST, answered.upstream.status());
    let error = answered
        .upstream
        .error()
        .ok_or("the 400 body decodes as an Error")?;
    assert_eq!(Some(90_001), error.code);
    Ok(())
}

#[tokio::test]
async fn query_aql_reports_the_408_execution_timeout() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query/aql"))
        .respond_with(ResponseTemplate::new(408))
        .mount(&server)
        .await;

    let answered = support::client(&server)?.query_aql(&request()).await?;
    assert!(matches!(
        answered.outcome,
        QueryExecuteAdhocQueryBodyOutcome::RequestTimeout
    ));
    Ok(())
}

#[tokio::test]
async fn query_aql_rows_walks_two_pages() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query/aql"))
        .and(body_string_contains("\"offset\":0"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"rows":[["row-1"],["row-2"]]}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/query/aql"))
        .and(body_string_contains("\"offset\":2"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"rows":[["row-3"]]}"#))
        .mount(&server)
        .await;

    let client = support::client(&server)?;
    let rows: Vec<Result<Vec<serde_json::Value>, QueryPageError>> = client
        .query_aql_rows(request(), PageSize::new(2)?)
        .collect()
        .await;
    let values: Vec<String> = rows
        .into_iter()
        .map(|row| {
            let row = row?;
            let first = row.first().ok_or("a row has at least one cell")?;
            Ok::<String, Box<dyn Error>>(first.as_str().unwrap_or_default().to_owned())
        })
        .collect::<Result<Vec<String>, Box<dyn Error>>>()?;
    assert_eq!(
        vec!["row-1".to_owned(), "row-2".to_owned(), "row-3".to_owned()],
        values
    );
    Ok(())
}

#[tokio::test]
async fn query_aql_rows_ends_the_stream_on_a_refusal() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/query/aql"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let client = support::client(&server)?;
    let rows: Vec<Result<Vec<serde_json::Value>, QueryPageError>> = client
        .query_aql_rows(request(), PageSize::new(2)?)
        .collect()
        .await;
    assert_eq!(1, rows.len());
    let first = rows.into_iter().next().ok_or("one item")?;
    match first {
        Err(QueryPageError::Refused(upstream)) => {
            assert_eq!(http::StatusCode::BAD_REQUEST, upstream.status());
        }
        other => return Err(format!("expected a refusal, got {other:?}").into()),
    }
    Ok(())
}
