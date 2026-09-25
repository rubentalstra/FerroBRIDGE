// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What can go wrong on the wire, as types that carry the upstream answer.
//!
//! A status the governing ITS-REST 1.1.0 operation documents is an outcome
//! variant of that call, never an error. Everything here is the other half: a
//! call that never reached a documented answer, and every one of these carries
//! the upstream status and body rather than an empty value.

use crate::ids::IdError;
use http::StatusCode;
use serde::Deserialize;
use url::Url;

/// The error body an openEHR service returns beside a `4xx` or `5xx` status.
///
/// Two shapes exist in ITS-REST 1.1.0 and this type reads either. The `OpenAPI`
/// `Error` schema requires `message` and `validationErrors`
/// (`ehr-codegen.openapi.yaml`, `components.schemas.Error`); the prose example
/// under §Requests and responses/HTTP status codes carries `message`, `code`
/// and `errors` instead. Absent members stay empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct OpenEhrError {
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

/// A non-success answer from the openEHR service, with what it said.
#[derive(Debug, Clone)]
pub struct UpstreamError {
    /// The HTTP status the service answered with.
    status: StatusCode,
    /// The response body, as received.
    body: String,
    /// The decoded error body, when the body was one.
    error: Option<OpenEhrError>,
}

impl UpstreamError {
    /// Returns the upstream answer `status` and `body` describe.
    #[must_use]
    pub fn new(status: StatusCode, body: String) -> Self {
        // NOTE: the OpenAPI documents make an error body optional ("The
        // response body MAY contain error details"), so a body that is not one
        // is legitimately absent rather than defective, and the raw body stays.
        let error = serde_json::from_str::<OpenEhrError>(&body)
            .ok()
            .filter(|decoded| decoded != &OpenEhrError::default());
        Self {
            status,
            body,
            error,
        }
    }

    /// Returns the HTTP status the service answered with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the response body, as received.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the decoded error body, when the service sent one.
    #[must_use]
    pub fn error(&self) -> Option<&OpenEhrError> {
        self.error.as_ref()
    }
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.error.as_ref().and_then(|error| error.message.as_ref()) {
            Some(message) => write!(f, "{} ({message})", self.status),
            None => write!(f, "{}", self.status),
        }
    }
}

/// A response body that could not be decoded as the route's media type.
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

/// A call into the openEHR service that did not reach a documented answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The request could not be built into a URL under the configured base.
    #[error("{base} cannot be a base URL for the openEHR REST API")]
    BaseUrl {
        /// The configured base URL.
        base: Url,
    },
    /// The HTTP client could not be built from the configuration.
    #[error("the HTTP client could not be built")]
    ClientBuild {
        /// What the HTTP stack reported.
        #[source]
        source: reqwest::Error,
    },
    /// The request never reached the service.
    #[error("the request to {url} could not be sent")]
    Transport {
        /// The URL that was called.
        url: Url,
        /// What the HTTP stack reported.
        #[source]
        source: reqwest::Error,
    },
    /// The service did not answer inside the configured timeout.
    #[error("the request to {url} timed out")]
    Timeout {
        /// The URL that was called.
        url: Url,
        /// What the HTTP stack reported.
        #[source]
        source: reqwest::Error,
    },
    /// The service answered `5xx`.
    #[error("the openEHR service answered {upstream} for {url}")]
    ServiceFailure {
        /// The URL that was called.
        url: Url,
        /// The upstream status and body.
        upstream: Box<UpstreamError>,
    },
    /// The service refused the credentials.
    #[error("the openEHR service refused the credentials for {url}")]
    Unauthorized {
        /// The URL that was called.
        url: Url,
        /// The `WWW-Authenticate` challenge, when the service sent one.
        challenge: Option<String>,
        /// The upstream status and body.
        upstream: Box<UpstreamError>,
    },
    /// The service answered a status this operation does not document.
    #[error(
        "the openEHR service answered {upstream} for {url}, which ITS-REST 1.1.0 does not document for this operation"
    )]
    UndocumentedStatus {
        /// The URL that was called.
        url: Url,
        /// The upstream status and body.
        upstream: Box<UpstreamError>,
    },
    /// The response body could not be decoded.
    #[error("the {media} body of {url} could not be decoded")]
    Body {
        /// The URL that was called.
        url: Url,
        /// The media type the route declares.
        media: &'static str,
        /// What the decoder reported.
        #[source]
        source: Box<BodyError>,
    },
    /// The service committed a contribution and its `201` body is neither
    /// schema the operation admits, so the uid is all the answer carries.
    #[error(
        "the openEHR service committed contribution {contribution_uid} and its answer body could not be decoded"
    )]
    CommittedBody {
        /// The contribution the `ETag` named.
        contribution_uid: crate::ids::ContributionUid,
        /// What the decoder reported.
        #[source]
        source: Box<BodyError>,
    },
    /// A response header the operation's answer relies on was absent.
    #[error("the {status} answer from {url} carries no {header} header")]
    MissingHeader {
        /// The URL that was called.
        url: Url,
        /// The status that was answered.
        status: StatusCode,
        /// The header that was expected.
        header: &'static str,
    },
    /// A response header carried something other than what it declares.
    #[error("the {header} header of {url} is not usable")]
    MalformedHeader {
        /// The URL that was called.
        url: Url,
        /// The header that was read.
        header: &'static str,
        /// Why its value could not be used.
        #[source]
        source: Box<IdError>,
    },
    /// A free-text part of a commit header carries a quote or a control
    /// character, which the quoted `key="value"` form cannot hold without
    /// changing the attribute list the service reads.
    #[error("an attribute of the {header} header carries text the quoted value form cannot hold")]
    HeaderAttribute {
        /// The header that was being built.
        header: &'static str,
        /// Which attribute, and which character.
        #[source]
        source: crate::commit::CodeError,
    },
    /// A value the caller supplied cannot travel in an HTTP header.
    #[error("the value for the {header} header is not a legal header value")]
    HeaderValue {
        /// The header that was being built.
        header: &'static str,
        /// What the HTTP stack reported.
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    /// The ADL 2 route answered a body that is not an operational template.
    #[error("the adl2 route answered a body whose _type is {found:?}, not OPERATIONAL_TEMPLATE")]
    NotOperationalTemplate {
        /// The `_type` the body declared, when it declared one.
        found: Option<String>,
    },
    /// An identifier the service returned is malformed.
    #[error("the openEHR service returned a malformed identifier")]
    Identifier(#[from] IdError),
}

impl Error {
    /// Returns the variant's name as a short kebab-case label, for a log line
    /// that must name the failure class and never the URL or a body.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BaseUrl { .. } => "base-url",
            Self::ClientBuild { .. } => "client-build",
            Self::Transport { .. } => "transport",
            Self::Timeout { .. } => "timeout",
            Self::ServiceFailure { .. } => "service-failure",
            Self::Unauthorized { .. } => "unauthorized",
            Self::UndocumentedStatus { .. } => "undocumented-status",
            Self::Body { .. } => "body",
            Self::CommittedBody { .. } => "committed-body",
            Self::MissingHeader { .. } => "missing-header",
            Self::MalformedHeader { .. } => "malformed-header",
            Self::HeaderAttribute { .. } => "header-attribute",
            Self::HeaderValue { .. } => "header-value",
            Self::NotOperationalTemplate { .. } => "not-operational-template",
            Self::Identifier(..) => "identifier",
        }
    }
    /// Returns whether a retry of the same request could succeed.
    ///
    /// Only a failure that did not reach the service, or one the service
    /// reported as its own (`5xx`), is transient. A documented refusal is an
    /// outcome rather than an error and never reaches this predicate.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport { .. } | Self::Timeout { .. } | Self::ServiceFailure { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, OpenEhrError, UpstreamError};
    use crate::ids::IdError;
    use http::StatusCode;

    #[test]
    fn the_openapi_error_shape_decodes() {
        let upstream = UpstreamError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"message":"no","validationErrors":["/content missing"]}"#.to_owned(),
        );
        let error = upstream.error().expect("the body is an error body");
        assert_eq!(Some("no"), error.message.as_deref());
        assert_eq!(vec!["/content missing".to_owned()], error.validation_errors);
    }

    #[test]
    fn the_prose_error_shape_decodes() {
        let upstream = UpstreamError::new(
            StatusCode::BAD_REQUEST,
            r#"{"message":"no","code":90000,"errors":[{"_type":"DV_CODED_TEXT"}]}"#.to_owned(),
        );
        let error = upstream.error().expect("the body is an error body");
        assert_eq!(Some(90_000), error.code);
        assert_eq!(1, error.errors.len());
    }

    #[test]
    fn a_body_that_is_not_an_error_body_stays_raw() {
        let upstream = UpstreamError::new(StatusCode::NOT_FOUND, "not found".to_owned());
        assert_eq!(None, upstream.error());
        assert_eq!("not found", upstream.body());
    }

    #[test]
    fn an_empty_json_object_is_not_an_error_body() {
        let upstream = UpstreamError::new(StatusCode::NOT_FOUND, "{}".to_owned());
        assert_eq!(None, upstream.error());
    }

    #[test]
    fn only_transport_timeout_and_server_errors_are_retried() {
        let server = Error::ServiceFailure {
            url: "http://example.invalid/v1/ehr"
                .parse()
                .expect("a valid URL"),
            upstream: Box::new(UpstreamError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                String::new(),
            )),
        };
        assert!(server.is_retryable());
        let undocumented = Error::UndocumentedStatus {
            url: "http://example.invalid/v1/ehr"
                .parse()
                .expect("a valid URL"),
            upstream: Box::new(UpstreamError::new(StatusCode::IM_A_TEAPOT, String::new())),
        };
        assert!(!undocumented.is_retryable());
        assert!(!Error::Identifier(IdError::Empty { kind: "ehr_id" }).is_retryable());
    }

    #[test]
    fn an_error_body_display_carries_the_message() {
        let upstream = UpstreamError::new(
            StatusCode::BAD_REQUEST,
            r#"{"message":"bad path"}"#.to_owned(),
        );
        assert_eq!("400 Bad Request (bad path)", upstream.to_string());
        assert_eq!(
            Some(&OpenEhrError {
                message: Some("bad path".to_owned()),
                validation_errors: Vec::new(),
                code: None,
                errors: Vec::new(),
            }),
            upstream.error()
        );
    }
}
