// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The COMPOSITION contract over the generated client: one case per
//! documented status, plus the wire form of `Prefer`, `If-Match` and the three
//! committal metadata headers the bridge adds.

use super::support;
use ferrobridge_server::cdr::ids::{EhrId, version_from_etag};
use ferrobridge_server::cdr::{Prefer, Returned, VersionAtTime};
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::rest::generated::ehr::client::{
    CompositionCreateOutcome, CompositionDeleteOutcome, CompositionGetOutcome,
    CompositionUpdateOutcome,
};
use openehr_rm::v1_2::composition::composition::Composition;
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

    let answered = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match answered.outcome {
        CompositionCreateOutcome::Created { body, headers } => {
            let version_id = ferrobridge_server::cdr::version_from_etag(
                "composition_create",
                http::StatusCode::CREATED,
                headers.etag.as_deref(),
            )?;
            assert_eq!(VERSION_1, version_id.value().to_owned());
            assert_eq!("1", version_id.version_tree_id().value());
            let returned = ferrobridge_server::cdr::returned::<Composition>(
                "composition_create",
                body.as_ref(),
                Prefer::Representation,
            )?;
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
async fn create_composition_reads_an_empty_201_as_minimal_whatever_was_preferred()
-> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(
            ResponseTemplate::new(201).insert_header("ETag", format!("W/\"{VERSION_1}\"")),
        )
        .mount(&server)
        .await;

    let answered = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match answered.outcome {
        CompositionCreateOutcome::Created { body, headers } => {
            let version_id = ferrobridge_server::cdr::version_from_etag(
                "composition_create",
                http::StatusCode::CREATED,
                headers.etag.as_deref(),
            )?;
            assert_eq!(VERSION_1, version_id.value().to_owned());
            let returned = ferrobridge_server::cdr::returned::<Composition>(
                "composition_create",
                body.as_ref(),
                Prefer::Representation,
            )?;
            assert!(matches!(returned, Returned::Minimal));
        }
        other => return Err(format!("expected a created composition, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn create_composition_reports_the_422_validation_errors() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(ferrobridge_testkit::stubs::its_rest::unprocessable(
            "the template does not validate the composition",
            &["/content[0]: unknown node"],
        ))
        .mount(&server)
        .await;

    let answered = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    assert!(
        matches!(
            answered.outcome,
            CompositionCreateOutcome::UnprocessableEntity { .. }
        ),
        "{:?}",
        answered.outcome
    );
    assert_eq!(
        http::StatusCode::UNPROCESSABLE_ENTITY,
        answered.upstream.status()
    );
    let error = answered
        .upstream
        .error()
        .ok_or("the 422 body decodes as an Error")?;
    assert_eq!(
        vec!["/content[0]: unknown node".to_owned()],
        error.validation_errors
    );
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

    let answered = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    assert!(matches!(
        answered.outcome,
        CompositionCreateOutcome::BadRequest { .. }
    ));
    Ok(())
}

#[tokio::test]
async fn create_composition_reads_a_prose_shape_400_from_the_outcome() -> Result<(), Box<dyn Error>>
{
    // NOTE: ITS-REST 1.1.0 §Requests and responses/HTTP status codes shows an
    // error body with `message`, `code` and `errors` and no `validationErrors`.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/composition")))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"message":"the composition body does not parse","code":90000,"errors":[]}"#,
        ))
        .mount(&server)
        .await;

    let answered = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match answered.outcome {
        CompositionCreateOutcome::BadRequest { body } => {
            assert_eq!(Some("the composition body does not parse"), body.message());
            assert!(body.validation_errors().is_empty());
            assert_eq!(
                Some(
                    r#"{"message":"the composition body does not parse","code":90000,"errors":[]}"#
                ),
                body.text()
            );
        }
        other => return Err(format!("expected the 400 outcome, got {other:?}").into()),
    }
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

    let answered = support::client(&server)?
        .create_composition(
            &EhrId::new(EHR)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    assert!(matches!(
        answered.outcome,
        CompositionCreateOutcome::NotFound { .. }
    ));
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

    let preceding = version_from_etag(&format!("W/\"{VERSION_1}\""))?;
    let answered = support::client(&server)?
        .update_composition(
            &EhrId::new(EHR)?,
            &HierObjectId::new(VERSIONED_OBJECT)?,
            &preceding,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match answered.outcome {
        CompositionUpdateOutcome::Ok { headers, .. } => {
            let version_id = ferrobridge_server::cdr::version_from_etag(
                "composition_update",
                http::StatusCode::OK,
                headers.etag.as_deref(),
            )?;
            assert_eq!(VERSION_2, version_id.value().to_owned());
        }
        other => return Err(format!("expected an updated composition, got {other:?}").into()),
    }
    assert_eq!(
        vec![format!("\"{VERSION_1}\"")],
        support::request_header(&server, 0, "if-match").await?
    );
    assert_eq!(
        vec!["Synthetic vital signs".to_owned()],
        support::request_header(&server, 0, "openehr-template-id").await?
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

    let answered = support::client(&server)?
        .update_composition(
            &EhrId::new(EHR)?,
            &HierObjectId::new(VERSIONED_OBJECT)?,
            &ObjectVersionId::new(VERSION_1)?,
            &support::composition()?,
            &support::commit_context()?,
            Prefer::Representation,
        )
        .await?;
    match answered.outcome {
        CompositionUpdateOutcome::PreconditionFailed { headers, .. } => {
            assert_eq!(
                http::StatusCode::PRECONDITION_FAILED,
                answered.upstream.status()
            );
            let latest = ferrobridge_server::cdr::optional_version_from_etag(
                "composition_update",
                headers.etag.as_deref(),
            )?;
            assert_eq!(
                Some(VERSION_2.to_owned()),
                latest.map(|id| id.value().to_owned())
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

    let answered = support::client(&server)?
        .composition(
            &EhrId::new(EHR)?,
            &UidBasedId::HierObjectId(HierObjectId::new(VERSIONED_OBJECT)?),
            None,
        )
        .await?;
    match answered.outcome {
        CompositionGetOutcome::Ok { body, headers } => {
            let version_id = ferrobridge_server::cdr::optional_version_from_etag(
                "composition_get",
                headers.etag.as_deref(),
            )?;
            assert_eq!(
                Some(VERSION_2.to_owned()),
                version_id.map(|id| id.value().to_owned())
            );
            assert_eq!(
                "openEHR-EHR-COMPOSITION.encounter.v1",
                body.archetype_node_id
            );
        }
        other => return Err(format!("expected a composition, got {other:?}").into()),
    }
    assert_eq!(
        vec!["return=representation".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
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

    let answered = support::client(&server)?
        .composition(
            &EhrId::new(EHR)?,
            &UidBasedId::HierObjectId(HierObjectId::new(VERSIONED_OBJECT)?),
            Some(&VersionAtTime::new("2026-09-12T10:00:00+02:00")?),
        )
        .await?;
    assert!(matches!(answered.outcome, CompositionGetOutcome::NoContent));
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

    let answered = support::client(&server)?
        .composition(
            &EhrId::new(EHR)?,
            &UidBasedId::ObjectVersionId(ObjectVersionId::new(VERSION_1)?),
            None,
        )
        .await?;
    assert!(matches!(
        answered.outcome,
        CompositionGetOutcome::NotFound { .. }
    ));
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

    let answered = support::client(&server)?
        .delete_composition(&EhrId::new(EHR)?, &ObjectVersionId::new(VERSION_1)?)
        .await?;
    match answered.outcome {
        CompositionDeleteOutcome::NoContent { headers } => {
            let version_id = ferrobridge_server::cdr::optional_version_from_etag(
                "composition_delete",
                headers.etag.as_deref(),
            )?;
            assert_eq!(
                Some(VERSION_2.to_owned()),
                version_id.map(|id| id.value().to_owned())
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

    let answered = support::client(&server)?
        .delete_composition(&EhrId::new(EHR)?, &ObjectVersionId::new(VERSION_1)?)
        .await?;
    match answered.outcome {
        CompositionDeleteOutcome::Conflict { headers, .. } => {
            assert_eq!(http::StatusCode::CONFLICT, answered.upstream.status());
            let latest = ferrobridge_server::cdr::optional_version_from_etag(
                "composition_delete",
                headers.etag.as_deref(),
            )?;
            assert_eq!(
                Some(VERSION_2.to_owned()),
                latest.map(|id| id.value().to_owned())
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

    let answered = support::client(&server)?
        .delete_composition(&EhrId::new(EHR)?, &ObjectVersionId::new(VERSION_1)?)
        .await?;
    assert!(matches!(
        answered.outcome,
        CompositionDeleteOutcome::BadRequest { .. }
    ));
    Ok(())
}
