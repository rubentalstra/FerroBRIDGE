// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION contract.

use crate::support;
use ferrobridge_openehr::contribution::CreateContributionOutcome;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::prefer::{Prefer, Returned};
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
