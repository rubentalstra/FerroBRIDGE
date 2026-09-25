// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION contract.

use crate::support;
use ferrobridge_openehr::contribution::{ContributionOutcome, CreateContributionOutcome};
use ferrobridge_openehr::ids::{ContributionUid, EhrId};
use ferrobridge_openehr::prefer::{Prefer, Returned};
use ferrobridge_testkit::stubs::its_rest;
use openehr_its::rest::generated::ehr::NewContribution;
use std::error::Error;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The EHR every case in this module commits into.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";
/// The contribution identifier the service answers with.
const CONTRIBUTION: &str = "0826851c-c4c2-4d61-92b9-410fb8275ff0";

/// A synthetic CONTRIBUTION envelope with one composition version.
const NEW_CONTRIBUTION_JSON: &str = r#"{
  "versions": [
    {
      "_type": "ORIGINAL_VERSION",
      "data": {
        "_type": "COMPOSITION",
        "name": {"_type": "DV_TEXT", "value": "Synthetic encounter"},
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "language": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_639-1"}, "code_string": "en"},
        "territory": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "ISO_3166-1"}, "code_string": "NL"},
        "category": {"_type": "DV_CODED_TEXT", "value": "event", "defining_code": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}, "code_string": "433"}},
        "composer": {"_type": "PARTY_IDENTIFIED", "name": "Synthetic Composer"}
      },
      "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "complete", "defining_code": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}, "code_string": "532"}},
      "commit_audit": {
        "_type": "UPDATE_AUDIT",
        "change_type": {"_type": "DV_CODED_TEXT", "value": "creation", "defining_code": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}, "code_string": "249"}},
        "committer": {"_type": "PARTY_IDENTIFIED", "name": "Synthetic Committer"}
      }
    }
  ],
  "audit": {
    "_type": "UPDATE_AUDIT",
    "change_type": {"_type": "DV_CODED_TEXT", "value": "creation", "defining_code": {"_type": "CODE_PHRASE", "terminology_id": {"_type": "TERMINOLOGY_ID", "value": "openehr"}, "code_string": "249"}},
    "committer": {"_type": "PARTY_IDENTIFIED", "name": "Synthetic Committer"}
  }
}"#;

/// Returns the synthetic contribution request body.
fn new_contribution() -> Result<NewContribution, Box<dyn Error>> {
    Ok(serde_json::from_str::<NewContribution>(
        NEW_CONTRIBUTION_JSON,
    )?)
}

#[tokio::test]
async fn create_contribution_reads_the_uid_out_of_the_201_etag() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/contribution")))
        .respond_with(
            ResponseTemplate::new(201).insert_header("ETag", format!("W/\"{CONTRIBUTION}\"")),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_contribution(&EhrId::new(EHR)?, &new_contribution()?, Prefer::Minimal)
        .await?;
    match outcome {
        CreateContributionOutcome::Created {
            contribution_uid,
            returned,
        } => {
            assert_eq!(CONTRIBUTION, contribution_uid.as_str());
            assert!(matches!(returned, Returned::Minimal));
        }
        other => return Err(format!("expected a created contribution, got {other:?}").into()),
    }
    assert_eq!(
        vec!["return=minimal".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn create_contribution_reports_the_400() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/contribution")))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"message":"a first version cannot be a MODIFICATION","validationErrors":[]}"#,
        ))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_contribution(&EhrId::new(EHR)?, &new_contribution()?, Prefer::Minimal)
        .await?;
    match outcome {
        CreateContributionOutcome::BadRequest(upstream) => {
            let error = upstream.error().ok_or("the 400 body decodes as an Error")?;
            assert_eq!(
                Some("a first version cannot be a MODIFICATION"),
                error.message.as_deref()
            );
        }
        other => return Err(format!("expected a bad request, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn create_contribution_reports_the_409() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/contribution")))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_contribution(&EhrId::new(EHR)?, &new_contribution()?, Prefer::Minimal)
        .await?;
    assert!(matches!(outcome, CreateContributionOutcome::Conflict(_)));
    Ok(())
}

#[tokio::test]
async fn create_contribution_reads_an_empty_201_as_minimal_whatever_it_preferred()
-> Result<(), Box<dyn Error>> {
    // "If the `Prefer` header is missing or set to `return=minimal`, the body
    // is empty" (`ehr-codegen.openapi.yaml`, `201_CONTRIBUTION`).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/contribution")))
        .respond_with(its_rest::contribution_created_minimal(CONTRIBUTION))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_contribution(
            &EhrId::new(EHR)?,
            &new_contribution()?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        CreateContributionOutcome::Created {
            contribution_uid,
            returned,
        } => {
            assert_eq!(CONTRIBUTION, contribution_uid.as_str());
            assert!(matches!(returned, Returned::Minimal));
        }
        other => return Err(format!("expected a created contribution, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn create_contribution_reads_an_identifier_201_under_representation()
-> Result<(), Box<dyn Error>> {
    // The `201` body is `oneOf` CONTRIBUTION and `Identifier`
    // (`ehr-codegen.openapi.yaml`, `201_CONTRIBUTION`).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/contribution")))
        .respond_with(its_rest::created(
            CONTRIBUTION,
            &format!("{}/v1/ehr/{EHR}/contribution/{CONTRIBUTION}", server.uri()),
            &format!(r#"{{"uid":"{CONTRIBUTION}"}}"#),
        ))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .create_contribution(
            &EhrId::new(EHR)?,
            &new_contribution()?,
            Prefer::Representation,
        )
        .await?;
    match outcome {
        CreateContributionOutcome::Created {
            returned: Returned::Identifier(identifier),
            ..
        } => assert_eq!(CONTRIBUTION, identifier.uid),
        other => return Err(format!("expected an identifier, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn create_contribution_names_the_committed_uid_when_the_201_body_is_neither_schema()
-> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/ehr/{EHR}/contribution")))
        .respond_with(its_rest::created(
            CONTRIBUTION,
            &format!("{}/v1/ehr/{EHR}/contribution/{CONTRIBUTION}", server.uri()),
            r#"{"_type":"CONTRIBUTION","uid":{"_type":"HIER_OBJECT_ID","value":"x"}}"#,
        ))
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .create_contribution(
            &EhrId::new(EHR)?,
            &new_contribution()?,
            Prefer::Representation,
        )
        .await
        .expect_err("a body that is neither schema is refused");
    match error {
        ferrobridge_openehr::error::Error::CommittedBody {
            contribution_uid, ..
        } => assert_eq!(CONTRIBUTION, contribution_uid.as_str()),
        other => return Err(format!("expected a committed-body error, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn contribution_reads_the_200_as_the_contribution_and_its_versions()
-> Result<(), Box<dyn Error>> {
    // `contribution_get` answers `200_CONTRIBUTION`, the CONTRIBUTION schema
    // (`ehr-codegen.openapi.yaml`).
    let server = MockServer::start().await;
    let versions = [
        "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1",
        "5f0e2b1a-0000-4000-8000-00000000000d::openEHRSys.example.com::1",
    ];
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/contribution/{CONTRIBUTION}")))
        .respond_with(its_rest::retrieved(&its_rest::contribution_body(
            CONTRIBUTION,
            &versions,
        )))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .contribution(&EhrId::new(EHR)?, &ContributionUid::new(CONTRIBUTION)?)
        .await?;
    let ContributionOutcome::Found(contribution) = outcome else {
        return Err(format!("expected the contribution, got {outcome:?}").into());
    };
    assert_eq!(CONTRIBUTION, contribution.uid.value());
    assert_eq!(versions.len(), contribution.versions.len());
    assert_eq!(
        vec!["application/json".to_owned()],
        support::request_header(&server, 0, "accept").await?
    );
    Ok(())
}

#[tokio::test]
async fn contribution_reports_the_404() -> Result<(), Box<dyn Error>> {
    // "`404 Not Found` is returned when an EHR with `ehr_id` does not exist,
    // or when a CONTRIBUTION with `contribution_uid` does not exist"
    // (`ehr-codegen.openapi.yaml`, `404_CONTRIBUTION`).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/contribution/{CONTRIBUTION}")))
        .respond_with(its_rest::not_found("no such contribution"))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .contribution(&EhrId::new(EHR)?, &ContributionUid::new(CONTRIBUTION)?)
        .await?;
    match outcome {
        ContributionOutcome::NotFound(upstream) => {
            let error = upstream.error().ok_or("the 404 body decodes as an Error")?;
            assert_eq!(Some("no such contribution"), error.message.as_deref());
        }
        other => return Err(format!("expected not found, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn contribution_refuses_a_200_body_that_is_no_contribution() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/contribution/{CONTRIBUTION}")))
        .respond_with(its_rest::retrieved(r#"{"_type":"CONTRIBUTION"}"#))
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .contribution(&EhrId::new(EHR)?, &ContributionUid::new(CONTRIBUTION)?)
        .await
        .expect_err("a body that is no CONTRIBUTION is refused");
    assert!(
        matches!(error, ferrobridge_openehr::error::Error::Body { .. }),
        "{error:?}"
    );
    Ok(())
}

#[tokio::test]
async fn contribution_retries_a_server_failure_as_a_get() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/contribution/{CONTRIBUTION}")))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/contribution/{CONTRIBUTION}")))
        .respond_with(its_rest::retrieved(&its_rest::contribution_body(
            CONTRIBUTION,
            &["8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::1"],
        )))
        .with_priority(2)
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .contribution(&EhrId::new(EHR)?, &ContributionUid::new(CONTRIBUTION)?)
        .await?;
    assert!(matches!(outcome, ContributionOutcome::Found(_)));
    assert_eq!(
        2,
        server
            .received_requests()
            .await
            .ok_or("the mock server records requests")?
            .len()
    );
    Ok(())
}

#[tokio::test]
async fn contribution_reports_a_status_the_operation_does_not_document()
-> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/ehr/{EHR}/contribution/{CONTRIBUTION}")))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .contribution(&EhrId::new(EHR)?, &ContributionUid::new(CONTRIBUTION)?)
        .await
        .expect_err("a 409 is no documented answer of contribution_get");
    match error {
        ferrobridge_openehr::error::Error::UndocumentedStatus { upstream, .. } => {
            assert_eq!(http::StatusCode::CONFLICT, upstream.status());
        }
        other => return Err(format!("expected an undocumented status, got {other:?}").into()),
    }
    Ok(())
}
