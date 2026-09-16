// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The COMPOSITION resource: create, retrieve, update and delete.
//!
//! Four facts of `docs/specs/its-rest/computable/OAS/ehr-codegen.openapi.yaml`
//! and the ITS-REST 1.1.0 prose shape this module. `Prefer` always travels.
//! `If-Match` carries the bare quoted `version_uid`. A concurrency failure is
//! `412` on `PUT` and `409` on `DELETE`. A `GET` answers `204` when the
//! composition was deleted at the requested time, which is its own outcome and
//! never an absent value.

use crate::client::{Call, Client, Idempotency, if_match_value};
use crate::commit::CommitContext;
use crate::decode;
use crate::error::{Error, UpstreamError};
use crate::ids::{EhrId, ObjectVersionId, VersionedObjectUid};
use crate::prefer::{Prefer, Returned};
use http::{Method, StatusCode};
use openehr_rm::v1_2::composition::composition::Composition;

/// A point in time a version is read at, in the extended ISO 8601 form.
///
/// The value travels as the `version_at_time` query parameter
/// (`ehr-codegen.openapi.yaml`, `components.parameters.version_at_time`); the
/// service interprets it, and the client only carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionAtTime(String);

impl VersionAtTime {
    /// Returns the instant `text` names.
    ///
    /// # Errors
    /// Returns [`crate::ids::IdError`] when `text` is empty or carries a
    /// control character.
    pub fn new(text: &str) -> Result<Self, crate::ids::IdError> {
        if text.is_empty() {
            return Err(crate::ids::IdError::Empty {
                kind: "version_at_time",
            });
        }
        if let Some(character) = text.chars().find(|c| c.is_control()) {
            return Err(crate::ids::IdError::ForbiddenCharacter {
                kind: "version_at_time",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the instant as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `uid_based_id` of a composition read.
///
/// "The `uid_based_id` can take a form of an `OBJECT_VERSION_ID` identifier
/// taken from `VERSION.uid.value` …, or a form of a `HIER_OBJECT_ID`
/// identifier taken from `VERSIONED_OBJECT.uid.value`"
/// (`ehr-codegen.openapi.yaml`, `composition_get`). The first addresses one
/// exact version, the second the latest, or the one extant at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UidBasedId {
    /// The version container, which addresses the latest version.
    VersionedObject(VersionedObjectUid),
    /// One exact version.
    Version(ObjectVersionId),
}

impl std::fmt::Display for UidBasedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionedObject(uid) => write!(f, "{uid}"),
            Self::Version(id) => write!(f, "{id}"),
        }
    }
}

/// What `POST /ehr/{ehr_id}/composition` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum CreateCompositionOutcome {
    /// `201`: the first version exists, and the `ETag` named its
    /// `version_uid`.
    Created {
        /// The identifier of the new version.
        version_id: ObjectVersionId,
        /// What the requested `Prefer` asked the service to return.
        returned: Returned<Composition>,
    },
    /// `400`: the request could not be parsed or is invalid.
    BadRequest(UpstreamError),
    /// `404`: no EHR with this `ehr_id` exists.
    UnknownEhr(UpstreamError),
    /// `422`: the content is well-formed and fails semantic validation.
    Unprocessable(UpstreamError),
}

/// What `PUT /ehr/{ehr_id}/composition/{uid_based_id}` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum UpdateCompositionOutcome {
    /// `200` or `204`: the new version exists, and the `ETag` named it.
    Updated {
        /// The identifier of the new version.
        version_id: ObjectVersionId,
        /// What the requested `Prefer` asked the service to return.
        returned: Returned<Composition>,
    },
    /// `400`: the request could not be parsed or is invalid.
    BadRequest(UpstreamError),
    /// `404`: no EHR or no version container with these identifiers exists.
    NotFound(UpstreamError),
    /// `412`: `If-Match` does not name the latest version.
    PreconditionFailed {
        /// The latest version the service reported in its `ETag`.
        latest_version_id: Option<ObjectVersionId>,
        /// The upstream status and body.
        upstream: UpstreamError,
    },
    /// `422`: the content is well-formed and fails semantic validation.
    Unprocessable(UpstreamError),
}

/// What `GET /ehr/{ehr_id}/composition/{uid_based_id}` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum CompositionOutcome {
    /// `200`: the composition, with the version the `ETag` named.
    Found {
        /// The version that was read, when the service sent an `ETag`.
        version_id: Option<ObjectVersionId>,
        /// The composition itself.
        composition: Box<Composition>,
    },
    /// `204`: the composition was deleted at the requested time.
    Deleted,
    /// `404`: no EHR, or no version of the composition at that time.
    NotFound(UpstreamError),
}

/// What `DELETE /ehr/{ehr_id}/composition/{preceding_version_uid}` answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum DeleteCompositionOutcome {
    /// `204`: the composition is deleted.
    Deleted {
        /// The version the service reported in its `ETag`.
        version_id: Option<ObjectVersionId>,
    },
    /// `400`: the request is invalid, or the composition is already deleted.
    BadRequest(UpstreamError),
    /// `404`: no EHR or no composition with these identifiers exists.
    NotFound(UpstreamError),
    /// `409`: the supplied `uid_based_id` is not the latest version.
    Conflict {
        /// The latest version the service reported in its `ETag`.
        latest_version_id: Option<ObjectVersionId>,
        /// The upstream status and body.
        upstream: UpstreamError,
    },
}

impl Client {
    /// Commits the first version of a composition into the EHR `ehr_id`.
    ///
    /// The body is canonical JSON, the mandatory composition representation of
    /// ITS-REST 1.1.0, and `commit` renders the committal metadata headers.
    /// The call is a `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, when the `201` carries no `ETag`, when the commit metadata
    /// cannot travel in a header, or when a body cannot be decoded.
    pub async fn create_composition(
        &self,
        ehr_id: &EhrId,
        composition: &Composition,
        commit: &CommitContext,
        prefer: Prefer,
    ) -> Result<CreateCompositionOutcome, Error> {
        let url = self.url(&["ehr", ehr_id.as_str(), "composition"])?;
        let mut call = Call::new(Method::POST, url, Idempotency::NonIdempotent)
            .preferring(prefer)
            .with_json_body(openehr_its::json::to_canonical_json(composition));
        call.commit_headers = commit.headers()?;
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::CREATED => Ok(CreateCompositionOutcome::Created {
                version_id: decode::version_id(&answer)?,
                returned: decode::returned::<Composition>(&answer, prefer)?,
            }),
            StatusCode::BAD_REQUEST => Ok(CreateCompositionOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND => Ok(CreateCompositionOutcome::UnknownEhr(answer.upstream())),
            StatusCode::UNPROCESSABLE_ENTITY => {
                Ok(CreateCompositionOutcome::Unprocessable(answer.upstream()))
            }
            _ => Err(answer.undocumented()),
        }
    }

    /// Commits a new version of the composition `versioned_object_uid`.
    ///
    /// `preceding` is the latest version the caller knows of; it travels as
    /// the `If-Match` header, quoted and without the `W/` an `ETag` carries.
    /// The call is idempotent under that precondition, so the retry budget
    /// applies.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, when a `2xx` carries no `ETag`, when the commit metadata
    /// cannot travel in a header, or when a body cannot be decoded.
    pub async fn update_composition(
        &self,
        ehr_id: &EhrId,
        versioned_object_uid: &VersionedObjectUid,
        preceding: &ObjectVersionId,
        composition: &Composition,
        commit: &CommitContext,
        prefer: Prefer,
    ) -> Result<UpdateCompositionOutcome, Error> {
        let url = self.url(&[
            "ehr",
            ehr_id.as_str(),
            "composition",
            versioned_object_uid.as_str(),
        ])?;
        let mut call = Call::new(Method::PUT, url, Idempotency::Idempotent)
            .preferring(prefer)
            .with_json_body(openehr_its::json::to_canonical_json(composition));
        call.if_match = Some(if_match_value(preceding));
        call.commit_headers = commit.headers()?;
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::OK => Ok(UpdateCompositionOutcome::Updated {
                version_id: decode::version_id(&answer)?,
                returned: decode::returned::<Composition>(&answer, prefer)?,
            }),
            StatusCode::NO_CONTENT => Ok(UpdateCompositionOutcome::Updated {
                version_id: decode::version_id(&answer)?,
                returned: Returned::Minimal,
            }),
            StatusCode::BAD_REQUEST => Ok(UpdateCompositionOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND => Ok(UpdateCompositionOutcome::NotFound(answer.upstream())),
            StatusCode::PRECONDITION_FAILED => Ok(UpdateCompositionOutcome::PreconditionFailed {
                latest_version_id: decode::optional_version_id(&answer)?,
                upstream: answer.upstream(),
            }),
            StatusCode::UNPROCESSABLE_ENTITY => {
                Ok(UpdateCompositionOutcome::Unprocessable(answer.upstream()))
            }
            _ => Err(answer.undocumented()),
        }
    }

    /// Retrieves a version of a composition.
    ///
    /// `at` selects the version extant at that time and is only meaningful
    /// when `uid_based_id` is a version container.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, or when the body cannot be decoded.
    pub async fn composition(
        &self,
        ehr_id: &EhrId,
        uid_based_id: &UidBasedId,
        at: Option<&VersionAtTime>,
    ) -> Result<CompositionOutcome, Error> {
        let mut url = self.url(&[
            "ehr",
            ehr_id.as_str(),
            "composition",
            &uid_based_id.to_string(),
        ])?;
        if let Some(at) = at {
            url.query_pairs_mut()
                .append_pair("version_at_time", at.as_str());
        }
        let call = Call::new(Method::GET, url, Idempotency::Idempotent);
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::OK => Ok(CompositionOutcome::Found {
                version_id: decode::optional_version_id(&answer)?,
                composition: Box::new(decode::canonical::<Composition>(&answer)?),
            }),
            StatusCode::NO_CONTENT => Ok(CompositionOutcome::Deleted),
            StatusCode::NOT_FOUND => Ok(CompositionOutcome::NotFound(answer.upstream())),
            _ => Err(answer.undocumented()),
        }
    }

    /// Deletes the composition whose latest version is `preceding`.
    ///
    /// "The `uid_based_id` MUST be in a form of an `OBJECT_VERSION_ID`
    /// identifier taken from the last (most recent) `VERSION.uid.value`"
    /// (`ehr-codegen.openapi.yaml`, `composition_delete`), which is why this
    /// call takes a version identifier and is retryable.
    ///
    /// # Errors
    /// Returns [`Error`] when the call did not reach one of the documented
    /// answers, or when an `ETag` is malformed.
    pub async fn delete_composition(
        &self,
        ehr_id: &EhrId,
        preceding: &ObjectVersionId,
    ) -> Result<DeleteCompositionOutcome, Error> {
        let url = self.url(&[
            "ehr",
            ehr_id.as_str(),
            "composition",
            &preceding.to_string(),
        ])?;
        let call = Call::new(Method::DELETE, url, Idempotency::Idempotent);
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::NO_CONTENT => Ok(DeleteCompositionOutcome::Deleted {
                version_id: decode::optional_version_id(&answer)?,
            }),
            StatusCode::BAD_REQUEST => Ok(DeleteCompositionOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND => Ok(DeleteCompositionOutcome::NotFound(answer.upstream())),
            StatusCode::CONFLICT => Ok(DeleteCompositionOutcome::Conflict {
                latest_version_id: decode::optional_version_id(&answer)?,
                upstream: answer.upstream(),
            }),
            _ => Err(answer.undocumented()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "test assertions")]

    use super::{UidBasedId, VersionAtTime};
    use crate::ids::{IdError, ObjectVersionId, VersionedObjectUid};

    #[test]
    fn a_uid_based_id_renders_either_form() -> Result<(), IdError> {
        assert_eq!(
            "8849182c",
            UidBasedId::VersionedObject(VersionedObjectUid::new("8849182c")?).to_string()
        );
        assert_eq!(
            "8849182c::system::1",
            UidBasedId::Version(ObjectVersionId::new("8849182c::system::1")?).to_string()
        );
        Ok(())
    }

    #[test]
    fn a_version_at_time_refuses_a_control_character() {
        assert!(VersionAtTime::new("2015-01-20T19:30:22\u{7}").is_err());
        assert!(VersionAtTime::new("").is_err());
    }
}
