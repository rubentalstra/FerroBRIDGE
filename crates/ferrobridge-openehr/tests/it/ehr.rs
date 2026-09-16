// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR contract: one case per status
//! `docs/specs/its-rest/computable/OAS/ehr-codegen.openapi.yaml` documents.

use crate::support;
use ferrobridge_openehr::ehr::{CreateEhrOutcome, EhrOutcome};
use ferrobridge_openehr::ids::{EhrId, SubjectId, SubjectNamespace};
use ferrobridge_openehr::prefer::{Prefer, Returned};
use std::error::Error;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn create_ehr_reads_the_ehr_id_out_of_the_201_etag() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ehr"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", "W/\"7d44b88c-4199-4bad-97dc-d78268e01398\"")
                .insert_header("Content-Type", "application/json")
                .set_body_string(support::EHR_JSON),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    match outcome {
        CreateEhrOutcome::Created { ehr_id, returned } => {
            assert_eq!("7d44b88c-4199-4bad-97dc-d78268e01398", ehr_id.as_str());
            assert!(matches!(returned, Returned::Representation(_)));
        }
        other => return Err(format!("expected a created EHR, got {other:?}").into()),
    }
    assert_eq!(
        vec!["return=representation".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn create_ehr_with_minimal_prefer_reads_no_body() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ehr"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", "W/\"7d44b88c-4199-4bad-97dc-d78268e01398\""),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_ehr(None, Prefer::Minimal)
        .await?;
    assert!(matches!(
        outcome,
        CreateEhrOutcome::Created {
            returned: Returned::Minimal,
            ..
        }
    ));
    assert_eq!(
        vec!["return=minimal".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn create_ehr_carries_the_400_body() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ehr"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"message":"malformed EHR_STATUS","validationErrors":["/subject missing"]}"#,
        ))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    match outcome {
        CreateEhrOutcome::BadRequest(upstream) => {
            assert_eq!(http::StatusCode::BAD_REQUEST, upstream.status());
            let error = upstream.error().ok_or("the 400 body decodes as an Error")?;
            assert_eq!(Some("malformed EHR_STATUS"), error.message.as_deref());
            assert_eq!(vec!["/subject missing".to_owned()], error.validation_errors);
        }
        other => return Err(format!("expected a bad request, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn create_ehr_reports_the_409_conflict() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ehr"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    assert!(matches!(outcome, CreateEhrOutcome::Conflict(_)));
    Ok(())
}

#[tokio::test]
async fn ehr_by_subject_sends_both_parameters() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/ehr"))
        .and(query_param("subject_id", "synthetic-ins01"))
        .and(query_param("subject_namespace", "synthetic-examples"))
        .respond_with(ResponseTemplate::new(200).set_body_string(support::EHR_JSON))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .ehr_by_subject(
            &SubjectId::new("synthetic-ins01")?,
            &SubjectNamespace::new("synthetic-examples")?,
        )
        .await?;
    match outcome {
        EhrOutcome::Found(ehr) => {
            assert_eq!("7d44b88c-4199-4bad-97dc-d78268e01398", ehr.ehr_id.value());
        }
        other => return Err(format!("expected an EHR, got {other:?}").into()),
    }
    assert_eq!(
        vec!["return=representation".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn ehr_by_subject_reports_the_404() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/ehr"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .ehr_by_subject(
            &SubjectId::new("synthetic-ins01")?,
            &SubjectNamespace::new("synthetic-examples")?,
        )
        .await?;
    assert!(matches!(outcome, EhrOutcome::NotFound(_)));
    Ok(())
}

#[tokio::test]
async fn ehr_by_id_reports_the_404() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/ehr/7d44b88c-4199-4bad-97dc-d78268e01398"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .ehr(&EhrId::new("7d44b88c-4199-4bad-97dc-d78268e01398")?)
        .await?;
    assert!(matches!(outcome, EhrOutcome::NotFound(_)));
    Ok(())
}
