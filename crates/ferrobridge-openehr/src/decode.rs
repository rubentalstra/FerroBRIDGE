// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading a response body and the identity headers beside it.

use crate::client::Answer;
use crate::error::{BodyError, Error};
use crate::prefer::{Prefer, Returned};
use http::header::ETAG;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_its::rest::generated::common::Identifier;
use serde::de::DeserializeOwned;

/// Returns the canonical JSON RM value the body carries.
///
/// Canonical JSON is the mandatory composition representation of ITS-REST
/// 1.1.0, and the reader is the strict one the openEHR crates provide.
pub(crate) fn canonical<T: DeserializeOwned>(answer: &Answer) -> Result<T, Error> {
    openehr_its::json::from_canonical_json::<T>(&answer.body).map_err(|source| Error::Body {
        url: answer.url.clone(),
        media: "canonical JSON",
        source: Box::new(BodyError::CanonicalJson(source)),
    })
}

/// Returns the `OpenAPI` schema value the body carries.
pub(crate) fn schema<T: DeserializeOwned>(answer: &Answer) -> Result<T, Error> {
    serde_json::from_str::<T>(&answer.body).map_err(|source| Error::Body {
        url: answer.url.clone(),
        media: "JSON",
        source: Box::new(BodyError::Json(source)),
    })
}

/// Returns what a `2xx` answer carries, per the `Prefer` it was asked with.
///
/// ITS-REST 1.1.0 pairs each preference with a body: no body for
/// `return=minimal`, the `Identifier` schema for `return=identifier`, and the
/// resource itself for `return=representation`. An empty body answers
/// [`Returned::Minimal`] whatever was asked, because the answer's status and
/// headers already name what the service stored.
pub(crate) fn returned<T: DeserializeOwned>(
    answer: &Answer,
    prefer: Prefer,
) -> Result<Returned<T>, Error> {
    // NOTE: RFC 7240 §2 lets a server ignore a preference it cannot honour, so
    // an empty body is the minimal answer and never a decoding failure.
    if answer.body.trim().is_empty() {
        return Ok(Returned::Minimal);
    }
    match prefer {
        Prefer::Minimal => Ok(Returned::Minimal),
        Prefer::Identifier => schema::<Identifier>(answer).map(Returned::Identifier),
        Prefer::Representation => {
            canonical::<T>(answer).map(|value| Returned::Representation(Box::new(value)))
        }
    }
}

/// Returns the version identifier the `ETag` of `answer` names.
pub(crate) fn version_id(answer: &Answer) -> Result<ObjectVersionId, Error> {
    let etag = answer
        .header(ETAG.as_str())
        .ok_or_else(|| Error::MissingHeader {
            url: answer.url.clone(),
            status: answer.status,
            header: "ETag",
        })?;
    crate::ids::version_from_etag(etag).map_err(|source| Error::MalformedHeader {
        url: answer.url.clone(),
        header: "ETag",
        source: Box::new(source),
    })
}

/// Returns the version identifier the `ETag` of `answer` names, when it sent
/// one.
///
/// The `412` and `409` answers SHOULD carry the latest `version_uid` in the
/// `ETag` (ITS-REST 1.1.0 §Requests and responses/HTTP headers/If-Match and
/// accidental overwrites), so an absent header is a legal answer and stays an
/// absent value rather than an error.
pub(crate) fn optional_version_id(answer: &Answer) -> Result<Option<ObjectVersionId>, Error> {
    match answer.header(ETAG.as_str()) {
        None => Ok(None),
        Some(etag) => crate::ids::version_from_etag(etag)
            .map(Some)
            .map_err(|source| Error::MalformedHeader {
                url: answer.url.clone(),
                header: "ETag",
                source: Box::new(source),
            }),
    }
}
