// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR resource: `POST /ehr`, `GET /ehr` by subject, `GET /ehr/{ehr_id}`.
//!
//! The statuses each outcome names are the ones
//! `docs/specs/its-rest/computable/OAS/ehr-codegen.openapi.yaml` documents for
//! the operation, and nothing else reaches a caller as a value.

use crate::client::{Answer, Call, Client, Idempotency};
use crate::decode;
use crate::error::{Error, UpstreamError};
use crate::ids::{EhrId, SubjectId, SubjectNamespace};
use crate::prefer::{Prefer, Returned};
use http::{Method, StatusCode};
use openehr_rm::v1_2::ehr::ehr::Ehr;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;

/// What `POST /ehr` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum CreateEhrOutcome {
    /// `201`: the EHR exists, and the `ETag` named its `ehr_id`.
    Created {
        /// The identifier of the new EHR.
        ehr_id: EhrId,
        /// What the requested `Prefer` asked the service to return.
        returned: Returned<Ehr>,
    },
    /// `400`: the request could not be parsed or is invalid.
    BadRequest(UpstreamError),
    /// `409`: an EHR for the same subject already exists.
    Conflict(UpstreamError),
}

/// What a `GET` of one EHR answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum EhrOutcome {
    /// `200`: the EHR.
    Found(Box<Ehr>),
    /// `404`: no EHR matches.
    NotFound(UpstreamError),
}

impl Client {
    /// Creates an EHR with a server-assigned identifier.
    ///
    /// `status` is the optional `EHR_STATUS` the service commits with the new
    /// EHR; when it is absent the service builds its default one
    /// (`ehr-codegen.openapi.yaml`, `ehr_create`). The call is a `POST`, so it
    /// is never retried.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, when the `201` carries no `ETag`, or when a body cannot be
    /// decoded.
    pub async fn create_ehr(
        &self,
        status: Option<&EhrStatus>,
        prefer: Prefer,
    ) -> Result<CreateEhrOutcome, Error> {
        let url = self.url(&["ehr"])?;
        let mut call = Call::new(Method::POST, url, Idempotency::NonIdempotent).preferring(prefer);
        if let Some(status) = status {
            call = call.with_json_body(openehr_its::json::to_canonical_json(status));
        }
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::CREATED => Ok(CreateEhrOutcome::Created {
                ehr_id: ehr_id_from_etag(&answer)?,
                returned: decode::returned::<Ehr>(&answer, prefer)?,
            }),
            StatusCode::BAD_REQUEST => Ok(CreateEhrOutcome::BadRequest(answer.upstream())),
            StatusCode::CONFLICT => Ok(CreateEhrOutcome::Conflict(answer.upstream())),
            _ => Err(answer.undocumented()),
        }
    }

    /// Retrieves the EHR with `ehr_id`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, or when the body cannot be decoded.
    pub async fn ehr(&self, ehr_id: &EhrId) -> Result<EhrOutcome, Error> {
        let url = self.url(&["ehr", ehr_id.as_str()])?;
        let call = Call::new(Method::GET, url, Idempotency::Idempotent);
        let answer = self.execute(call).await?;
        ehr_outcome(&answer)
    }

    /// Retrieves the EHR whose subject is `subject_id` in `namespace`.
    ///
    /// The parameters are matched against
    /// `EHR_STATUS.subject.external_ref.id.value` and
    /// `EHR_STATUS.subject.external_ref.namespace`
    /// (`ehr-codegen.openapi.yaml`, `ehr_get_by_subject`).
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, or when the body cannot be decoded.
    pub async fn ehr_by_subject(
        &self,
        subject_id: &SubjectId,
        namespace: &SubjectNamespace,
    ) -> Result<EhrOutcome, Error> {
        let mut url = self.url(&["ehr"])?;
        url.query_pairs_mut()
            .append_pair("subject_id", subject_id.as_str())
            .append_pair("subject_namespace", namespace.as_str());
        let call = Call::new(Method::GET, url, Idempotency::Idempotent);
        let answer = self.execute(call).await?;
        ehr_outcome(&answer)
    }
}

/// Returns the outcome of a `GET` that answers `200_EHR` or `404`.
fn ehr_outcome(answer: &Answer) -> Result<EhrOutcome, Error> {
    match answer.status {
        StatusCode::OK => {
            decode::canonical::<Ehr>(answer).map(|ehr| EhrOutcome::Found(Box::new(ehr)))
        }
        StatusCode::NOT_FOUND => Ok(EhrOutcome::NotFound(answer.upstream())),
        _ => Err(answer.undocumented()),
    }
}

/// Returns the `ehr_id` the `ETag` of a `201_EHR` answer names.
///
/// "The `ETag` (i.e. entity tag) response header is the `ehr_id` identifier,
/// enclosed by double quotes" (`ehr-codegen.openapi.yaml`,
/// `components.headers.ETag_EHR`).
fn ehr_id_from_etag(answer: &Answer) -> Result<EhrId, Error> {
    let etag = answer
        .header(http::header::ETAG.as_str())
        .ok_or_else(|| Error::MissingHeader {
            url: answer.url.clone(),
            status: answer.status,
            header: "ETag",
        })?;
    EhrId::new(crate::ids::entity_tag(etag)).map_err(|source| Error::MalformedHeader {
        url: answer.url.clone(),
        header: "ETag",
        source: Box::new(source),
    })
}
