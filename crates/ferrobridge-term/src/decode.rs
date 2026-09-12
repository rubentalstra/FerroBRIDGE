// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Turning one answer into one outcome, for a single call and for a batch
//! entry alike.
//!
//! A successful operation answers "a `Parameters` resource"; a failed one
//! answers "an `OperationOutcome` resource with error details" under a `4xx`
//! or `5xx` (<https://hl7.org/fhir/R4/operations.html>). Which refusal means
//! "this code is not known" is the one decision no FHIR clause fixes, and
//! [`is_not_found`] states it.

use crate::client::Answer;
use crate::config::WireVersion;
use crate::error::{Error, UpstreamError};
use crate::outcome::{BatchOutcome, LookupOutcome, Request, TranslateOutcome, ValidateOutcome};
use http::StatusCode;

/// The `tx-issue-type` codes that say the server does not know the concept.
///
/// No specification governs this mapping: our own design. The FHIR operations
/// framework fixes neither the status nor the issue code a terminology server
/// uses for a code it cannot find, so the client reads the machine-readable
/// classification the terminology ecosystem defines
/// (<https://build.fhir.org/ig/FHIR/fhir-tools-ig/CodeSystem-tx-issue-type.html>)
/// and falls back to a bare `404`.
const NOT_FOUND_TX_ISSUES: [&str; 2] = ["not-found", "invalid-code"];

/// Returns the `CodeSystem/$lookup` outcome `answer` carries.
pub(crate) fn lookup(version: WireVersion, answer: &Answer) -> Result<LookupOutcome, Error> {
    if answer.status.is_success() {
        return crate::wire::lookup_found(version, &answer.body)
            .map_err(|source| body(crate::LOOKUP, answer, source));
    }
    let upstream = UpstreamError::new(version, answer.status, answer.body.clone());
    if is_not_found(&upstream) {
        return Ok(LookupOutcome::NotFound {
            outcome: Box::new(upstream),
        });
    }
    Err(refusal(crate::LOOKUP, answer, upstream))
}

/// Returns the `ConceptMap/$translate` outcome `answer` carries.
///
/// A refusal is an error rather than a missing translation: passing a source
/// code through under a target system, or writing nothing where the server
/// could not answer, would both assert something the concept map never said.
pub(crate) fn translate(version: WireVersion, answer: &Answer) -> Result<TranslateOutcome, Error> {
    if answer.status.is_success() {
        return crate::wire::translate(version, &answer.body)
            .map_err(|source| body(crate::TRANSLATE, answer, source));
    }
    let upstream = UpstreamError::new(version, answer.status, answer.body.clone());
    Err(refusal(crate::TRANSLATE, answer, upstream))
}

/// Returns the `ValueSet/$validate-code` outcome `answer` carries.
pub(crate) fn validate_code(
    version: WireVersion,
    answer: &Answer,
) -> Result<ValidateOutcome, Error> {
    if answer.status.is_success() {
        return crate::wire::validate_code(version, &answer.body)
            .map_err(|source| body(crate::VALIDATE_CODE, answer, source));
    }
    let upstream = UpstreamError::new(version, answer.status, answer.body.clone());
    Err(refusal(crate::VALIDATE_CODE, answer, upstream))
}

/// Returns the outcome one batch entry carries, in the shape of its request.
pub(crate) fn batch_entry(
    version: WireVersion,
    request: &Request,
    answer: &Answer,
) -> Result<BatchOutcome, Error> {
    match request {
        Request::Lookup { .. } => lookup(version, answer).map(BatchOutcome::Lookup),
        Request::Translate { .. } => translate(version, answer).map(BatchOutcome::Translate),
        Request::ValidateCode { .. } => {
            validate_code(version, answer).map(BatchOutcome::ValidateCode)
        }
    }
}

/// Returns whether `upstream` says the concept is not known.
fn is_not_found(upstream: &UpstreamError) -> bool {
    if NOT_FOUND_TX_ISSUES
        .iter()
        .any(|code| upstream.has_tx_issue(code))
    {
        return true;
    }
    upstream.status() == StatusCode::NOT_FOUND && upstream.outcome().is_none()
}

/// Returns the error for a refusal the operation cannot read as an outcome.
fn refusal(operation: &'static str, answer: &Answer, upstream: UpstreamError) -> Error {
    let url = answer.url.clone();
    let upstream = Box::new(upstream);
    if answer.status == StatusCode::UNAUTHORIZED {
        return Error::Unauthorized {
            url,
            challenge: None,
            upstream,
        };
    }
    if answer.status.is_server_error() {
        return Error::ServerFailure { url, upstream };
    }
    Error::Refused {
        operation,
        url,
        upstream,
    }
}

/// Returns the error for a body the operation cannot decode.
fn body(operation: &'static str, answer: &Answer, source: crate::error::BodyError) -> Error {
    Error::Body {
        operation,
        url: answer.url.clone(),
        source: Box::new(source),
    }
}
