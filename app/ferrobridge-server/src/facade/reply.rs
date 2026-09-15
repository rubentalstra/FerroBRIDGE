// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Writing a facade answer onto the wire.
//!
//! Every response carries `Content-Type: application/fhir+json`
//! (<https://hl7.org/fhir/R4/http.html#mime-type>), and every body is either a
//! FHIR resource or an `OperationOutcome` the facade authored
//! (`docs/architecture.md` §4.6). A refusal that cannot be encoded is still a
//! refusal: it answers `500` with a plain outcome rather than a truncated
//! document.

use axum::body::Body;
use axum::response::IntoResponse;
use axum::response::Response;
use fhir_types::codec::Json;
use fhir_types::codec::Object;
use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use http::header;

use crate::facade::media::FHIR_JSON;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::outcome;

/// One header a facade answer adds beside the media type.
#[derive(Debug, Clone)]
pub struct Header {
    /// The header name.
    name: HeaderName,
    /// The value it carries.
    value: String,
}

impl Header {
    /// Returns the `ETag` of a version, in the weak form R4 writes.
    ///
    /// "Servers … SHALL return an `ETag` header … `W/"3"`"
    /// (<https://hl7.org/fhir/R4/http.html#versioning>).
    #[must_use]
    pub fn entity_tag(version_id: &str) -> Self {
        Self {
            name: header::ETAG,
            value: format!("W/\"{version_id}\""),
        }
    }

    /// Returns the `Location` of a created resource.
    ///
    /// "The `Location` header … `[base]/[type]/[id]/_history/[vid]`"
    /// (<https://hl7.org/fhir/R4/http.html#create>).
    #[must_use]
    pub fn location(url: &str) -> Self {
        Self {
            name: header::LOCATION,
            value: url.to_owned(),
        }
    }

    /// Returns the `WWW-Authenticate` challenge an upstream sent.
    #[must_use]
    pub fn challenge(value: &str) -> Self {
        Self {
            name: header::WWW_AUTHENTICATE,
            value: value.to_owned(),
        }
    }
}

/// Returns the response carrying `body` under `status`, with `headers`.
///
/// A header value the HTTP stack refuses is dropped rather than failing the
/// answer: the body already states the outcome, and an `ETag` that cannot
/// travel is a weaker answer, never a wrong one.
#[must_use]
pub fn resource(status: StatusCode, body: &Object, headers: &[Header]) -> Response {
    match serde_json::to_vec(body) {
        Ok(bytes) => assemble(status, bytes, headers),
        Err(error) => {
            tracing::error!(
                error = error.to_string(),
                "a facade body could not be serialized"
            );
            internal_failure()
        }
    }
}

/// Returns the response carrying `issues` under `status`, with `headers`.
#[must_use]
pub fn issues(status: StatusCode, issues: &[Issue], headers: &[Header]) -> Response {
    match outcome(issues).to_json() {
        Ok(object) => resource(status, &object, headers),
        Err(error) => {
            tracing::error!(
                error = error.to_string(),
                "an OperationOutcome could not be encoded"
            );
            internal_failure()
        }
    }
}

/// Returns the response carrying one issue under `status`.
#[must_use]
pub fn issue(status: StatusCode, one: Issue) -> Response {
    issues(status, &[one], &[])
}

/// Returns an empty `200` answer with no body.
///
/// `Prefer: return=minimal` asks for "an empty payload"
/// (<https://hl7.org/fhir/R4/http.html#return>), so the body is absent and the
/// headers still carry `ETag` and `Location`.
#[must_use]
pub fn minimal(status: StatusCode, headers: &[Header]) -> Response {
    assemble(status, Vec::new(), headers)
}

/// Builds the response from its parts.
fn assemble(status: StatusCode, bytes: Vec<u8>, headers: &[Header]) -> Response {
    let mut response = Response::builder().status(status);
    if !bytes.is_empty() {
        response = response.header(header::CONTENT_TYPE, FHIR_JSON);
    }
    for entry in headers {
        match HeaderValue::from_str(&entry.value) {
            Ok(value) => response = response.header(entry.name.clone(), value),
            Err(error) => tracing::warn!(
                header = entry.name.as_str(),
                error = error.to_string(),
                "a response header value could not travel and was dropped"
            ),
        }
    }
    match response.body(Body::from(bytes)) {
        Ok(built) => built,
        Err(error) => {
            tracing::error!(
                error = error.to_string(),
                "a facade response could not be built"
            );
            internal_failure()
        }
    }
}

/// Returns the `500` a response that cannot be built falls back to.
///
/// The body is a fixed `OperationOutcome` written as bytes, so the fallback
/// cannot itself fail to encode.
fn internal_failure() -> Response {
    let body = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": crate::facade::outcome::Severity::Error.as_str(),
            "code": IssueType::Exception.as_str(),
            "diagnostics": "the server could not write its own answer",
        }],
    })
    .to_string();
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, FHIR_JSON)],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{Header, issue, issues, minimal, resource};
    use crate::facade::outcome::{Issue, IssueType};
    use fhir_types::codec::{Object, Value};
    use http::{StatusCode, header};

    #[test]
    fn every_body_carrying_answer_declares_the_fhir_media_type() {
        let mut body = Object::new();
        body.insert(
            String::from("resourceType"),
            Value::String(String::from("Condition")),
        );
        let response = resource(StatusCode::OK, &body, &[]);
        assert_eq!(
            Some("application/fhir+json"),
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
        );
    }

    #[test]
    fn the_entity_tag_is_the_weak_form_r4_writes() {
        let response = resource(
            StatusCode::CREATED,
            &Object::new(),
            &[Header::entity_tag("2"), Header::location("http://x/y")],
        );
        assert_eq!(
            Some("W/\"2\""),
            response
                .headers()
                .get(header::ETAG)
                .and_then(|value| value.to_str().ok())
        );
        assert_eq!(
            Some("http://x/y"),
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
        );
    }

    #[test]
    fn a_minimal_answer_carries_no_body_and_no_media_type() {
        let response = minimal(StatusCode::OK, &[Header::entity_tag("1")]);
        assert!(response.headers().get(header::CONTENT_TYPE).is_none());
        assert!(response.headers().get(header::ETAG).is_some());
    }

    #[test]
    fn an_outcome_answer_keeps_its_status() {
        let response = issue(
            StatusCode::NOT_FOUND,
            Issue::error(IssueType::NotFound).diagnosing("no such id"),
        );
        assert_eq!(StatusCode::NOT_FOUND, response.status());
        let many = issues(
            StatusCode::UNPROCESSABLE_ENTITY,
            &[Issue::error(IssueType::Invalid)],
            &[],
        );
        assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, many.status());
    }
}
