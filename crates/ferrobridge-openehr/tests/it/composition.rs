// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The COMPOSITION contract: one case per documented status, plus the wire
//! form of `Prefer`, `If-Match` and the three committal metadata headers.

use crate::support;
use ferrobridge_openehr::composition::{
    CompositionOutcome, CreateCompositionOutcome, DeleteCompositionOutcome, UidBasedId,
    UpdateCompositionOutcome, VersionAtTime,
};
use ferrobridge_openehr::ids::{EhrId, ObjectVersionId, VersionedObjectUid};
use ferrobridge_openehr::prefer::{Prefer, Returned};
use std::error::Error;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The EHR every case in this module commits into.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";
/// The version container every case in this module addresses.
const VERSIONED_OBJECT: &str = "8849182c-82ad-4088-a07f-48ead4180515";
/// The first version of that container.
const VERSION_1: &str = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1";
/// The second version of that container.
const VERSION_2: &str = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2";

#[tokio::test]
async fn create_composition_sends_the_three_commit_headers() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", format!("W/\"{VERSION_1}\""))
                .set_body_string(support::COMPOSITION_JSON),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        CreateCompositionOutcome::Created {
            version_id,
            returned,
        } => {
            assert_eq!(VERSION_1, version_id.to_string());
            assert_eq!("1", version_id.version_tree_id());
            assert!(matches!(returned, Returned::Representation(_)));
        }
        other => return Err(format!("expected a created composition, got {other:?}").into()),
    }

    assert_eq!(
        vec!["lifecycle_state.code_string=\"532\"".to_owned()],
        support::request_header(&server, 0, "openehr-version").await?
    );
    assert_eq!(
        vec![
            "change_type.code_string=\"251\"".to_owned(),
            "description.value=\"A synthetic commit\"".to_owned(),
            "committer.name=\"Synthetic Committer\",committer.external_ref.id=\"bc8132ea-0000-4000-8000-000000000003\",committer.external_ref.namespace=\"demographic\",committer.external_ref.type=\"PERSON\"".to_owned(),
        ],
        support::request_header(&server, 0, "openehr-audit-details").await?
    );
    assert_eq!(
        vec!["Synthetic vital signs".to_owned()],
        support::request_header(&server, 0, "openehr-template-id").await?
    );
    assert_eq!(
        vec!["application/json".to_owned()],
        support::request_header(&server, 0, "content-type").await?
    );
    assert_eq!(
        vec!["return=representation".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn create_composition_reports_the_422_validation_errors() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(ResponseTemplate::new(422).set_body_string(
            r#"{"message":"the template does not validate the composition","validationErrors":["/content[0]: unknown node"]}"#,
        ))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        CreateCompositionOutcome::Unprocessable(upstream) => {
            assert_eq!(http::StatusCode::UNPROCESSABLE_ENTITY, upstream.status());
            let error = upstream.error().ok_or("the 422 body decodes as an Error")?;
            assert_eq!(
                vec!["/content[0]: unknown node".to_owned()],
                error.validation_errors
            );
        }
        other => return Err(format!("expected an unprocessable entity, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn create_composition_reports_the_400() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    assert!(matches!(outcome, CreateCompositionOutcome::BadRequest(_)));
    Ok(())
}

#[tokio::test]
async fn create_composition_reports_the_unknown_ehr() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    assert!(matches!(outcome, CreateCompositionOutcome::UnknownEhr(_)));
    Ok(())
}

#[tokio::test]
async fn update_composition_sends_the_bare_quoted_if_match() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/v1/ehr/{EHR}/composition/{VERSIONED_OBJECT}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_2}\""))
                .set_body_string(support::COMPOSITION_JSON),
        )
        .mount(&server)
        .await;

    let preceding = ObjectVersionId::from_etag(&format!("W/\"{VERSION_1}\""))?;
    let outcome = support::client(&server)?
        .update_composition(
            &EhrId::new(EHR)?,
            &VersionedObjectUid::new(VERSIONED_OBJECT)?,
            &preceding,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        UpdateCompositionOutcome::Updated { version_id, .. } => {
            assert_eq!(VERSION_2, version_id.to_string());
        }
        other => return Err(format!("expected an updated composition, got {other:?}").into()),
    }
    assert_eq!(
        vec![format!("\"{VERSION_1}\"")],
        support::request_header(&server, 0, "if-match").await?
    );
    Ok(())
}

#[tokio::test]
async fn update_composition_reports_the_412_with_the_latest_version() -> Result<(), Box<dyn Error>>
{
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/v1/ehr/{EHR}/composition/{VERSIONED_OBJECT}"
        )))
        .respond_with(
            ResponseTemplate::new(412).insert_header("ETag", format!("W/\"{VERSION_2}\"")),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .update_composition(
            &EhrId::new(EHR)?,
            &VersionedObjectUid::new(VERSIONED_OBJECT)?,
            &ObjectVersionId::new(VERSION_1)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        UpdateCompositionOutcome::PreconditionFailed {
            latest_version_id,
            upstream,
        } => {
            assert_eq!(http::StatusCode::PRECONDITION_FAILED, upstream.status());
            assert_eq!(
                Some(VERSION_2.to_owned()),
                latest_version_id.map(|id| id.to_string())
            );
        }
        other => return Err(format!("expected a precondition failure, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn composition_reads_the_latest_version_of_a_container() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/v1/ehr/{EHR}/composition/{VERSIONED_OBJECT}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_2}\""))
                .set_body_string(support::COMPOSITION_JSON),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .composition(
            &EhrId::new(EHR)?,
            &UidBasedId::VersionedObject(VersionedObjectUid::new(VERSIONED_OBJECT)?),
            None,
        )
        .await?;
    match outcome {
        CompositionOutcome::Found {
            version_id,
            composition,
        } => {
            assert_eq!(
                Some(VERSION_2.to_owned()),
                version_id.map(|id| id.to_string())
            );
            assert_eq!(
                "openEHR-EHR-COMPOSITION.encounter.v1",
                composition.archetype_node_id
            );
        }
        other => return Err(format!("expected a composition, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn composition_at_a_time_answers_deleted_on_a_204() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/v1/ehr/{EHR}/composition/{VERSIONED_OBJECT}"
        )))
        .and(query_param("version_at_time", "2026-09-12T10:00:00+02:00"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .composition(
            &EhrId::new(EHR)?,
            &UidBasedId::VersionedObject(VersionedObjectUid::new(VERSIONED_OBJECT)?),
            Some(&VersionAtTime::new("2026-09-12T10:00:00+02:00")?),
        )
        .await?;
    assert!(matches!(outcome, CompositionOutcome::Deleted));
    Ok(())
}

#[tokio::test]
async fn composition_by_exact_version_reports_the_404() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/composition/{VERSION_1}")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .composition(
            &EhrId::new(EHR)?,
            &UidBasedId::Version(ObjectVersionId::new(VERSION_1)?),
            None,
        )
        .await?;
    assert!(matches!(outcome, CompositionOutcome::NotFound(_)));
    Ok(())
}

#[tokio::test]
async fn delete_composition_answers_deleted_on_a_204() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/ehr/{EHR}/composition/{VERSION_1}")))
        .respond_with(
            ResponseTemplate::new(204).insert_header("ETag", format!("W/\"{VERSION_2}\"")),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .delete_composition(&EhrId::new(EHR)?, &ObjectVersionId::new(VERSION_1)?)
        .await?;
    match outcome {
        DeleteCompositionOutcome::Deleted { version_id } => {
            assert_eq!(
                Some(VERSION_2.to_owned()),
                version_id.map(|id| id.to_string())
            );
        }
        other => return Err(format!("expected a deletion, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn delete_composition_reports_a_concurrency_failure_as_409() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/ehr/{EHR}/composition/{VERSION_1}")))
        .respond_with(
            ResponseTemplate::new(409).insert_header("ETag", format!("W/\"{VERSION_2}\"")),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .delete_composition(&EhrId::new(EHR)?, &ObjectVersionId::new(VERSION_1)?)
        .await?;
    match outcome {
        DeleteCompositionOutcome::Conflict {
            latest_version_id,
            upstream,
        } => {
            assert_eq!(http::StatusCode::CONFLICT, upstream.status());
            assert_eq!(
                Some(VERSION_2.to_owned()),
                latest_version_id.map(|id| id.to_string())
            );
        }
        other => return Err(format!("expected a conflict, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn delete_composition_reports_an_already_deleted_400() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/ehr/{EHR}/composition/{VERSION_1}")))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .delete_composition(&EhrId::new(EHR)?, &ObjectVersionId::new(VERSION_1)?)
        .await?;
    assert!(matches!(outcome, DeleteCompositionOutcome::BadRequest(_)));
    Ok(())
}
