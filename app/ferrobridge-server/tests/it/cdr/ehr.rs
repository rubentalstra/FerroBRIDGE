// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR contract over the generated client: one case per status
//! `docs/specs/its-rest/computable/OAS/ehr-codegen.openapi.yaml` documents,
//! plus what the bridge reads out of each answer.

use super::support;
use ferrobridge_server::cdr::ids::{EhrId, SubjectId, SubjectNamespace};
use ferrobridge_server::cdr::{Prefer, Returned};
use openehr_its::rest::generated::ehr::client::{
    EhrCreateOutcome, EhrGetByIdOutcome, EhrGetBySubjectOutcome,
};
use openehr_rm::v1_2::ehr::ehr::Ehr;
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

    let answered = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    match answered.outcome {
        EhrCreateOutcome::Created { body, headers } => {
            let ehr_id = ferrobridge_server::cdr::ehr_id_from_etag(
                http::StatusCode::CREATED,
                headers.etag.as_deref(),
            )?;
            assert_eq!("7d44b88c-4199-4bad-97dc-d78268e01398", ehr_id.as_str());
            let returned = ferrobridge_server::cdr::returned::<Ehr>(
                "ehr_create",
                body.as_ref(),
                Prefer::Representation,
            )?;
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

    let answered = support::client(&server)?
        .create_ehr(None, Prefer::Minimal)
        .await?;
    match answered.outcome {
        EhrCreateOutcome::Created { body, .. } => {
            let returned = ferrobridge_server::cdr::returned::<Ehr>(
                "ehr_create",
                body.as_ref(),
                Prefer::Minimal,
            )?;
            assert!(matches!(returned, Returned::Minimal));
        }
        other => return Err(format!("expected a created EHR, got {other:?}").into()),
    }
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

    let answered = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    assert!(
        matches!(answered.outcome, EhrCreateOutcome::BadRequest { .. }),
        "{:?}",
        answered.outcome
    );
    assert_eq!(http::StatusCode::BAD_REQUEST, answered.upstream.status());
    let error = answered
        .upstream
        .error()
        .ok_or("the 400 body decodes as an Error")?;
    assert_eq!(Some("malformed EHR_STATUS"), error.message.as_deref());
    assert_eq!(vec!["/subject missing".to_owned()], error.validation_errors);
    Ok(())
}

#[tokio::test]
async fn create_ehr_keeps_a_400_body_in_the_prose_error_shape() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ehr"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"message":"malformed EHR_STATUS","code":90000}"#),
        )
        .mount(&server)
        .await;

    let answered = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    assert!(
        matches!(
            &answered.outcome,
            EhrCreateOutcome::BadRequest { body }
                if body.message() == Some("malformed EHR_STATUS")
                    && body.validation_errors().is_empty()
        ),
        "{:?}",
        answered.outcome
    );
    let error = answered
        .upstream
        .error()
        .ok_or("the 400 body is read in its prose shape")?;
    assert_eq!(Some("malformed EHR_STATUS"), error.message.as_deref());
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

    let answered = support::client(&server)?
        .create_ehr(None, Prefer::Representation)
        .await?;
    assert!(matches!(
        answered.outcome,
        EhrCreateOutcome::Conflict { .. }
    ));
    assert_eq!(http::StatusCode::CONFLICT, answered.upstream.status());
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

    let answered = support::client(&server)?
        .ehr_by_subject(
            &SubjectId::new("synthetic-ins01")?,
            &SubjectNamespace::new("synthetic-examples")?,
        )
        .await?;
    match answered.outcome {
        EhrGetBySubjectOutcome::Ok { body, .. } => {
            assert_eq!("7d44b88c-4199-4bad-97dc-d78268e01398", body.ehr_id.value());
        }
        other @ EhrGetBySubjectOutcome::NotFound { .. } => {
            return Err(format!("expected an EHR, got {other:?}").into());
        }
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

    let answered = support::client(&server)?
        .ehr_by_subject(
            &SubjectId::new("synthetic-ins01")?,
            &SubjectNamespace::new("synthetic-examples")?,
        )
        .await?;
    assert!(matches!(
        answered.outcome,
        EhrGetBySubjectOutcome::NotFound { .. }
    ));
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

    let answered = support::client(&server)?
        .ehr(&EhrId::new("7d44b88c-4199-4bad-97dc-d78268e01398")?)
        .await?;
    assert!(matches!(
        answered.outcome,
        EhrGetByIdOutcome::NotFound { .. }
    ));
    Ok(())
}
