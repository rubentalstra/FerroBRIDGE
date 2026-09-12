// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION resource: one atomic commit of several versions.

use crate::client::{Call, Client, Idempotency};
use crate::decode;
use crate::error::{Error, UpstreamError};
use crate::ids::{ContributionUid, EhrId, entity_tag};
use crate::prefer::{Prefer, Returned};
use http::{Method, StatusCode};
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_rm::v1_2::common::change_control::contribution::Contribution;

/// What `POST /ehr/{ehr_id}/contribution` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum CreateContributionOutcome {
    /// `201`: the contribution exists, and the `ETag` named its
    /// `contribution_uid`.
    Created {
        /// The identifier of the new contribution.
        contribution_uid: ContributionUid,
        /// What the requested `Prefer` asked the service to return.
        returned: Returned<Contribution>,
    },
    /// `400`: the request is invalid, or a modification type does not match
    /// the operation.
    BadRequest(UpstreamError),
    /// `404`: no EHR with this `ehr_id` exists.
    UnknownEhr(UpstreamError),
    /// `409`: a resource with the same identifier already exists.
    Conflict(UpstreamError),
}

impl Client {
    /// Commits a CONTRIBUTION into the EHR `ehr_id`.
    ///
    /// The versions and their audits travel in the body, which is why this
    /// call sends no committal metadata header: the `NewContribution` schema
    /// carries `audit` and each `versions[i].commit_audit` itself
    /// (`ehr-codegen.openapi.yaml`, `contribution_create`). The call is a
    /// `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, when the `201` carries no `ETag`, or when a body cannot be
    /// decoded.
    pub async fn create_contribution(
        &self,
        ehr_id: &EhrId,
        contribution: &NewContribution,
        prefer: Prefer,
    ) -> Result<CreateContributionOutcome, Error> {
        let url = self.url(&["ehr", ehr_id.as_str(), "contribution"])?;
        let body = serde_json::to_string(contribution).map_err(|source| Error::Body {
            url: url.clone(),
            media: "JSON",
            source: Box::new(crate::error::BodyError::Json(source)),
        })?;
        let call = Call::new(Method::POST, url, Idempotency::NonIdempotent)
            .preferring(prefer)
            .with_json_body(body);
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::CREATED => {
                let etag = answer.header(http::header::ETAG.as_str()).ok_or_else(|| {
                    Error::MissingHeader {
                        url: answer.url.clone(),
                        status: answer.status,
                        header: "ETag",
                    }
                })?;
                let contribution_uid =
                    ContributionUid::new(entity_tag(etag)).map_err(|source| {
                        Error::MalformedHeader {
                            url: answer.url.clone(),
                            header: "ETag",
                            source: Box::new(source),
                        }
                    })?;
                Ok(CreateContributionOutcome::Created {
                    contribution_uid,
                    returned: decode::returned::<Contribution>(&answer, prefer)?,
                })
            }
            StatusCode::BAD_REQUEST => Ok(CreateContributionOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND => Ok(CreateContributionOutcome::UnknownEhr(answer.upstream())),
            StatusCode::CONFLICT => Ok(CreateContributionOutcome::Conflict(answer.upstream())),
            _ => Err(answer.undocumented()),
        }
    }
}
