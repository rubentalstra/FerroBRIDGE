// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION resource: one atomic commit of several versions, and its
//! read back.

use crate::client::{Answer, Call, Client, Idempotency};
use crate::decode;
use crate::error::{BodyError, Error, UpstreamError};
use crate::ids::{ContributionUid, EhrId, entity_tag};
use crate::prefer::{Prefer, Returned};
use http::{Method, StatusCode};
use openehr_its::rest::generated::common::Identifier;
use openehr_its::rest::generated::ehr::{ContributionGetParams, NewContribution};
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
        /// What the service returned, which is [`Returned::Minimal`] for an
        /// empty body whatever `Prefer` asked for.
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

/// What `GET /ehr/{ehr_id}/contribution/{contribution_uid}` answered.
///
/// The operation documents `200` and `404` (`ehr-codegen.openapi.yaml`,
/// `contribution_get`).
#[derive(Debug)]
#[non_exhaustive]
pub enum ContributionOutcome {
    /// `200`: the CONTRIBUTION, whose `versions` reference every version it
    /// committed (`200_CONTRIBUTION`).
    Found(Box<Contribution>),
    /// `404`: no EHR with this `ehr_id`, or no CONTRIBUTION with this
    /// `contribution_uid` (`404_CONTRIBUTION`).
    NotFound(UpstreamError),
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
    /// The `201` body is the CONTRIBUTION or its `Identifier`, and it is empty
    /// under `return=minimal` (`201_CONTRIBUTION`). An empty body answers
    /// [`Returned::Minimal`] whatever `prefer` asked for, so a service that
    /// did not honour the preference still names the contribution it
    /// committed.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, or when the `201` carries no `ETag`. Returns
    /// [`Error::CommittedBody`], naming the committed contribution, when a
    /// `201` body is neither schema `201_CONTRIBUTION` admits.
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
            source: Box::new(BodyError::Json(source)),
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
                let returned = committed(&answer, &contribution_uid, prefer)?;
                Ok(CreateContributionOutcome::Created {
                    contribution_uid,
                    returned,
                })
            }
            StatusCode::BAD_REQUEST => Ok(CreateContributionOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND => Ok(CreateContributionOutcome::UnknownEhr(answer.upstream())),
            StatusCode::CONFLICT => Ok(CreateContributionOutcome::Conflict(answer.upstream())),
            _ => Err(answer.undocumented()),
        }
    }

    /// Retrieves the CONTRIBUTION `contribution_uid` of the EHR `ehr_id`.
    ///
    /// The path is the one `contribution_get` declares, filled from its
    /// generated parameters, and the body is the canonical CONTRIBUTION
    /// envelope (`ehr-codegen.openapi.yaml`, `contribution_get`). The call is
    /// a `GET`, so the retry budget applies.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, or when the `200` body is no CONTRIBUTION.
    pub async fn contribution(
        &self,
        ehr_id: &EhrId,
        contribution_uid: &ContributionUid,
    ) -> Result<ContributionOutcome, Error> {
        let params = ContributionGetParams {
            ehr_id: ehr_id.as_str().to_owned(),
            contribution_uid: contribution_uid.as_str().to_owned(),
            accept: None,
        };
        let url = self.url(&[
            "ehr",
            &params.ehr_id,
            "contribution",
            &params.contribution_uid,
        ])?;
        let call = Call::new(Method::GET, url, Idempotency::Idempotent);
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::OK => {
                let contribution = decode::canonical::<Contribution>(&answer)?;
                Ok(ContributionOutcome::Found(Box::new(contribution)))
            }
            StatusCode::NOT_FOUND => Ok(ContributionOutcome::NotFound(answer.upstream())),
            _ => Err(answer.undocumented()),
        }
    }
}

/// Returns what the `201` of a committed contribution carries.
///
/// The schema is `oneOf` CONTRIBUTION and `Identifier`, and the body is empty
/// for `return=minimal` (`ehr-codegen.openapi.yaml`, `201_CONTRIBUTION`), so
/// the body is read as whichever of the three it is. The commit already
/// happened, so a body that is none of them is an [`Error::CommittedBody`]
/// naming the contribution.
fn committed(
    answer: &Answer,
    contribution_uid: &ContributionUid,
    prefer: Prefer,
) -> Result<Returned<Contribution>, Error> {
    if prefer == Prefer::Minimal || answer.body.trim().is_empty() {
        return Ok(Returned::Minimal);
    }
    match openehr_its::json::from_canonical_json::<Contribution>(&answer.body) {
        Ok(contribution) => Ok(Returned::Representation(Box::new(contribution))),
        // NOTE: `201_CONTRIBUTION` is `oneOf` CONTRIBUTION and `Identifier`, so
        // a body that is no CONTRIBUTION is legitimately read as the other.
        Err(first) => serde_json::from_str::<Identifier>(&answer.body)
            .map(Returned::Identifier)
            .map_err(|_identifier| Error::CommittedBody {
                contribution_uid: contribution_uid.clone(),
                source: Box::new(BodyError::CanonicalJson(first)),
            }),
    }
}
