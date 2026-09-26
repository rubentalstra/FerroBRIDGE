// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The contribution operations: commit and read back.

use http::HeaderMap;
use http::StatusCode;
use openehr_its::rest::client::ClientError;
use openehr_its::rest::generated::ehr::ContributionCreateParams;
use openehr_its::rest::generated::ehr::ContributionGetParams;
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::generated::ehr::client::ContributionCreateOutcome;
use openehr_its::rest::generated::ehr::client::ContributionGetOutcome;
use openehr_its::rest::generated::ehr::client::EhrClient;

use crate::cdr::error::CdrError;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::EhrId;

use super::Answered;
use super::CdrClient;
use super::Prefer;
use super::answered_by;
use super::contribution_uid_from_etag;
use super::representation;

impl CdrClient {
    /// Commits a CONTRIBUTION into the EHR `ehr_id`.
    ///
    /// The versions and their audits travel in the body, which is why this
    /// call sends no committal metadata header: the `NewContribution` schema
    /// carries `audit` and each `versions[i].commit_audit` itself
    /// (`ehr-codegen.openapi.yaml`, `contribution_create`). The call is a
    /// `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError::CommittedBody`], naming the committed contribution,
    /// when a `201` body is not JSON, and [`CdrError`] otherwise when the call
    /// did not reach a documented answer.
    pub async fn create_contribution(
        &self,
        ehr_id: &EhrId,
        contribution: &NewContribution,
        prefer: Prefer,
    ) -> Result<Answered<ContributionCreateOutcome>, CdrError> {
        let client = self.call(HeaderMap::new())?;
        let params = ContributionCreateParams {
            ehr_id: String::from(ehr_id.as_str()),
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_template_id: None,
        };
        let answered = EhrClient::new(&client)
            .contribution_create(&params, contribution)
            .await;
        match answered {
            Err(source @ ClientError::Body { status, .. }) if status == StatusCode::CREATED => {
                let upstream = client.transport().answered();
                let etag = upstream
                    .as_ref()
                    .and_then(|upstream| upstream.header("etag"));
                let contribution_uid =
                    contribution_uid_from_etag("contribution_create", status, etag.as_deref())?;
                Err(CdrError::CommittedBody {
                    contribution_uid,
                    source: Box::new(source),
                })
            }
            answered => answered_by(&client, answered),
        }
    }

    /// Retrieves the CONTRIBUTION `contribution_uid` of the EHR `ehr_id`
    /// (`ehr-codegen.openapi.yaml`, `contribution_get`). The call is a `GET`,
    /// so the retry budget applies.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn contribution(
        &self,
        ehr_id: &EhrId,
        contribution_uid: &ContributionUid,
    ) -> Result<Answered<ContributionGetOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = ContributionGetParams {
            ehr_id: String::from(ehr_id.as_str()),
            contribution_uid: String::from(contribution_uid.as_str()),
            accept: None,
        };
        let answered = EhrClient::new(&client).contribution_get(&params).await;
        answered_by(&client, answered)
    }
}
