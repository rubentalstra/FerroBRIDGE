// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a CDR call answered beside its outcome, and what went wrong when it
//! reached no documented answer.
//!
//! A status the governing ITS-REST 1.1.0 operation documents is a variant of
//! the generated outcome enum of that call. Everything here is the other half,
//! and every refusal keeps the upstream status and body instead of an empty
//! value.

use http::HeaderMap;
use http::StatusCode;
use openehr_its::rest::client::ClientError;
use serde::Deserialize;

use crate::cdr::commit::HeaderError;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::IdError;

/// The error body an openEHR service returns beside a `4xx` or `5xx` status.
///
/// Two shapes exist in ITS-REST 1.1.0 and this reader takes either. The
/// `OpenAPI` `Error` schema requires `message` and `validationErrors`
/// (`ehr-codegen.openapi.yaml`, `components.schemas.Error`); the prose example
/// under §Requests and responses/HTTP status codes carries `message`, `code`
/// and `errors` instead. The generated `Error` DTO requires both members of the
/// first shape, so it refuses the second.
// TODO(#104): read the generated `Error` DTO once the two error shapes agree
// upstream, and drop this reader.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ErrorBody {
    /// The human-readable message both shapes carry.
    pub message: Option<String>,
    /// The `validationErrors` of the `OpenAPI` `Error` schema.
    #[serde(rename = "validationErrors")]
    pub validation_errors: Vec<String>,
    /// The `code` of the prose shape.
    pub code: Option<i64>,
    /// The `errors` of the prose shape, whose example members are
    /// `DV_CODED_TEXT` values.
    pub errors: Vec<serde_json::Value>,
}

/// The status, headers and body one CDR answer carried.
///
/// The body can be clinical content, so the `Debug` rendering names its length
/// and never its text.
#[derive(Clone)]
pub struct Upstream {
    /// The HTTP status the service answered with.
    status: StatusCode,
    /// The response headers.
    headers: HeaderMap,
    /// The response body, as received.
    body: String,
    /// The decoded error body, when the body was one.
    error: Option<ErrorBody>,
}

impl Upstream {
    /// Returns the upstream answer `status`, `headers` and `body` describe.
    #[must_use]
    pub fn new(status: StatusCode, headers: HeaderMap, body: String) -> Self {
        // NOTE: the OpenAPI documents make an error body optional ("The
        // response body MAY contain error details"), so a body that is not one
        // is legitimately absent rather than defective, and the raw body stays.
        let error = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .filter(|decoded| decoded != &ErrorBody::default());
        Self {
            status,
            headers,
            body,
            error,
        }
    }

    /// Returns the HTTP status the service answered with.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the value of response header `name`, when it is usable text.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<String> {
        // NOTE: RFC 9110 §5.5 admits opaque octets in a field value; one that is
        // not visible ASCII is legitimately unusable as text, not defective.
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// Returns the response body, as received.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the decoded error body, when the service sent one.
    #[must_use]
    pub const fn error(&self) -> Option<&ErrorBody> {
        self.error.as_ref()
    }
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upstream")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.error.as_ref().and_then(|error| error.message.as_ref()) {
            Some(message) => write!(f, "{} ({message})", self.status),
            None => write!(f, "{}", self.status),
        }
    }
}

/// A response body the bridge could not decode as the shape its route
/// declares.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BodyError {
    /// Canonical JSON that is not the RM value the route declares.
    #[error(transparent)]
    CanonicalJson(#[from] openehr_its::json::JsonParseError),
    /// JSON that is not the `OpenAPI` schema the route declares.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// XML that is not a canonical OPT 1.4 operational template.
    #[error(transparent)]
    Xml(#[from] openehr_its::xml::runtime::XmlError),
}

/// A CDR call that did not reach a documented answer the bridge can use.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CdrError {
    /// The HTTP engine could not be built from the configuration.
    #[error("the HTTP engine of the CDR client could not be built")]
    Build {
        /// What `reqwest` reported.
        #[source]
        source: reqwest::Error,
    },
    /// The generated client refused the call, or the call reached no
    /// documented answer.
    #[error(
        "the CDR call did not reach a documented answer{}",
        answered_status(upstream.as_deref())
    )]
    Client {
        /// What the generated client reported.
        #[source]
        source: Box<ClientError>,
        /// The answer the service sent, when the call reached it.
        upstream: Option<Box<Upstream>>,
    },
    /// The commit metadata cannot travel in a header.
    #[error("the commit metadata cannot travel in a header")]
    Commit(#[from] HeaderError),
    /// A value the bridge supplied cannot travel in an HTTP header.
    #[error("the value for the {header} header is not a legal header value")]
    HeaderValue {
        /// The header that was being built.
        header: &'static str,
        /// What the HTTP stack reported.
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    /// A response header the answer relies on was absent.
    #[error("the {status} answer of {operation} carries no {header} header")]
    MissingHeader {
        /// The ITS-REST `operationId`.
        operation: &'static str,
        /// The status that was answered.
        status: StatusCode,
        /// The header that was expected.
        header: &'static str,
    },
    /// A response header carried something other than what it declares.
    #[error("the {header} header of the {operation} answer is not usable")]
    MalformedHeader {
        /// The ITS-REST `operationId`.
        operation: &'static str,
        /// The header that was read.
        header: &'static str,
        /// Why its value could not be used.
        #[source]
        source: Box<IdError>,
    },
    /// A documented body could not be decoded as the shape its preference or
    /// route selects.
    #[error("the body of the {operation} answer could not be decoded")]
    Body {
        /// The ITS-REST `operationId`.
        operation: &'static str,
        /// What the decoder reported.
        #[source]
        source: Box<BodyError>,
    },
    /// The service committed a contribution and its `201` body is no schema
    /// the operation admits, so the uid is all the answer carries.
    #[error(
        "the openEHR service committed contribution {contribution_uid} and its answer body could not be decoded"
    )]
    CommittedBody {
        /// The contribution the `ETag` named.
        contribution_uid: ContributionUid,
        /// What the decoder reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The ADL 2 route answered a body that is not an operational template.
    #[error("the adl2 route answered a body whose _type is {found:?}, not OPERATIONAL_TEMPLATE")]
    NotOperationalTemplate {
        /// The `_type` the body declared, when it declared one.
        found: Option<String>,
    },
    /// The engine read an answer and kept no record of it.
    #[error("the CDR answer was read and not recorded")]
    Unrecorded,
}

impl CdrError {
    /// Returns the refusal `source` is, with the answer the service sent.
    #[must_use]
    pub fn client(source: ClientError, upstream: Option<Upstream>) -> Self {
        Self::Client {
            source: Box::new(source),
            upstream: upstream.map(Box::new),
        }
    }

    /// Returns the variant's name as a short kebab-case label, for a log line
    /// or a diagnostic that must name the failure class and never a body.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Build { .. } => "client-build",
            Self::Client { source, .. } => client_kind(source),
            Self::Commit(HeaderError::Attribute { .. }) => "header-attribute",
            Self::Commit(_) | Self::HeaderValue { .. } => "header-value",
            Self::MissingHeader { .. } => "missing-header",
            Self::MalformedHeader { .. } => "malformed-header",
            Self::Body { .. } => "body",
            Self::CommittedBody { .. } => "committed-body",
            Self::NotOperationalTemplate { .. } => "not-operational-template",
            Self::Unrecorded => "unrecorded",
        }
    }
}

/// Returns the status clause of a refusal's display, when the call reached
/// the service.
fn answered_status(upstream: Option<&Upstream>) -> String {
    upstream.map_or_else(String::new, |upstream| {
        format!(": the CDR answered {}", upstream.status())
    })
}

/// Returns the kebab-case label of one generated-client refusal.
pub(crate) fn client_kind(error: &ClientError) -> &'static str {
    match error {
        ClientError::BaseUrl { .. } => "base-url",
        ClientError::Transport { .. } => "transport",
        ClientError::Build { .. } => "request-build",
        ClientError::HeaderName { .. } | ClientError::HeaderValue { .. } => "header-value",
        ClientError::UnsupportedMediaType { .. } => "media-type",
        ClientError::Serialize { .. } | ClientError::Body { .. } => "body",
        ClientError::Unauthorized { .. } => "unauthorized",
        ClientError::Forbidden { .. } => "forbidden",
        ClientError::ServiceFailure { .. } => "service-failure",
        ClientError::UndocumentedStatus { .. } => "undocumented-status",
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorBody, Upstream};
    use http::{HeaderMap, StatusCode};

    fn upstream(status: StatusCode, body: &str) -> Upstream {
        Upstream::new(status, HeaderMap::new(), body.to_owned())
    }

    #[test]
    fn the_openapi_error_shape_decodes() {
        let upstream = upstream(
            StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"message":"no","validationErrors":["/content missing"]}"#,
        );
        let error = upstream.error().expect("the body is an error body");
        assert_eq!(Some("no"), error.message.as_deref());
        assert_eq!(vec!["/content missing".to_owned()], error.validation_errors);
    }

    #[test]
    fn the_prose_error_shape_decodes() {
        let upstream = upstream(
            StatusCode::BAD_REQUEST,
            r#"{"message":"no","code":90000,"errors":[{"_type":"DV_CODED_TEXT"}]}"#,
        );
        let error = upstream.error().expect("the body is an error body");
        assert_eq!(Some(90_000), error.code);
        assert_eq!(1, error.errors.len());
    }

    #[test]
    fn a_body_that_is_not_an_error_body_stays_raw() {
        let upstream = upstream(StatusCode::NOT_FOUND, "not found");
        assert_eq!(None, upstream.error());
        assert_eq!("not found", upstream.body());
    }

    #[test]
    fn an_empty_json_object_is_not_an_error_body() {
        assert_eq!(None, upstream(StatusCode::NOT_FOUND, "{}").error());
    }

    #[test]
    fn an_error_body_display_carries_the_message() {
        let upstream = upstream(StatusCode::BAD_REQUEST, r#"{"message":"bad path"}"#);
        assert_eq!("400 Bad Request (bad path)", upstream.to_string());
        assert_eq!(
            Some(&ErrorBody {
                message: Some("bad path".to_owned()),
                validation_errors: Vec::new(),
                code: None,
                errors: Vec::new(),
            }),
            upstream.error()
        );
    }

    #[test]
    fn the_debug_rendering_never_carries_the_body() {
        let upstream = upstream(
            StatusCode::UNPROCESSABLE_ENTITY,
            "synthetic-clinical-marker",
        );
        assert!(!format!("{upstream:?}").contains("synthetic-clinical-marker"));
    }
}
