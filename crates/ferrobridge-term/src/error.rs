// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! What can go wrong on the wire, as types that carry the upstream answer.
//!
//! A refusal a terminology operation expresses in its own `out` parameters is
//! an outcome of that call (`crate::outcome`), never an error. Everything here
//! is the other half, and every one of these carries the upstream status and
//! body rather than an empty value.

use crate::config::WireVersion;
use http::StatusCode;
use url::Url;

/// The `tx-issue-type` code system of the FHIR tools implementation guide
/// (<https://build.fhir.org/ig/FHIR/fhir-tools-ig/CodeSystem-tx-issue-type.html>).
pub const TX_ISSUE_TYPE: &str = "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type";

/// The terminology-specific classification of one issue.
///
/// A terminology server states why a call failed in machine-readable form as a
/// `details.coding` from [`TX_ISSUE_TYPE`], beside the `issue.code` that every
/// `OperationOutcome` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxIssue {
    /// The code, for example `not-found` or `invalid-code`.
    pub code: String,
    /// The display the server sent with the code, when it sent one.
    pub display: Option<String>,
}

/// One issue of an `OperationOutcome`
/// (<https://hl7.org/fhir/R4/operationoutcome.html>).
///
/// `severity` and `code` stay as the server spelled them: the raw body is kept
/// beside this value in [`UpstreamError`], and a code outside the FHIR value
/// set would otherwise cost the whole diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// `issue.severity`, one of `fatal`, `error`, `warning`, `information`.
    pub severity: String,
    /// `issue.code`, a code of the FHIR `IssueType` value set.
    pub code: String,
    /// `issue.details.text`, when the server sent one.
    pub details: Option<String>,
    /// `issue.diagnostics`, when the server sent one.
    pub diagnostics: Option<String>,
    /// The [`TX_ISSUE_TYPE`] coding of `issue.details.coding`, when one is
    /// present.
    pub tx_issue_type: Option<TxIssue>,
}

/// A decoded `OperationOutcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The issues, in the order the server sent them.
    pub issues: Vec<Issue>,
}

impl Outcome {
    /// Returns every [`TX_ISSUE_TYPE`] coding the outcome carries.
    pub fn tx_issues(&self) -> impl Iterator<Item = &TxIssue> {
        self.issues
            .iter()
            .filter_map(|issue| issue.tx_issue_type.as_ref())
    }

    /// Returns whether any issue carries the [`TX_ISSUE_TYPE`] code `code`.
    #[must_use]
    pub fn has_tx_issue(&self, code: &str) -> bool {
        self.tx_issues().any(|issue| issue.code == code)
    }
}

/// A non-success answer from the terminology server, with what it said.
#[derive(Debug, Clone)]
pub struct UpstreamError {
    /// The HTTP status the server answered with.
    status: StatusCode,
    /// The response body, as received.
    body: String,
    /// The decoded `OperationOutcome`, when the body was one.
    outcome: Option<Outcome>,
}

impl UpstreamError {
    /// Returns the upstream answer `status` and `body` describe, read in
    /// `version`.
    ///
    /// A failed operation answers "an `OperationOutcome` resource with error
    /// details" (<https://hl7.org/fhir/R4/operations.html>), so the body is
    /// decoded as one; a body that is not one is a legal answer from a server
    /// that failed before the operation ran, and the raw body stays either
    /// way.
    #[must_use]
    pub fn new(version: WireVersion, status: StatusCode, body: String) -> Self {
        let outcome = crate::wire::outcome(version, &body).ok();
        Self {
            status,
            body,
            outcome,
        }
    }

    /// Returns the HTTP status the server answered with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the response body, as received.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the decoded `OperationOutcome`, when the body was one.
    #[must_use]
    pub fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    /// Returns every [`TX_ISSUE_TYPE`] coding the answer carries.
    pub fn tx_issues(&self) -> impl Iterator<Item = &TxIssue> {
        self.outcome.iter().flat_map(Outcome::tx_issues)
    }

    /// Returns whether any issue carries the [`TX_ISSUE_TYPE`] code `code`.
    #[must_use]
    pub fn has_tx_issue(&self, code: &str) -> bool {
        self.tx_issues().any(|issue| issue.code == code)
    }
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.issues.first())
        {
            Some(issue) => write!(f, "{} ({})", self.status, issue.code),
            None => write!(f, "{}", self.status),
        }
    }
}

/// A response body that is not the resource the operation answers with.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BodyError {
    /// The body is not the FHIR JSON resource the route declares.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The `Parameters` resource does not fit the operation's declared
    /// parameter set.
    #[error(transparent)]
    Parameters(#[from] fhir_types::operation::ParametersError),
    /// A decoded value has no FHIR JSON form, so it could not be read back.
    #[error(transparent)]
    Encode(#[from] fhir_types::codec::EncodeError),
    /// A parameter the operation declares as mandatory states no value.
    #[error("the answer states no value for the mandatory parameter `{parameter}`")]
    MissingValue {
        /// The parameter, dotted for a part.
        parameter: &'static str,
    },
    /// The answer to a `batch` is not a `batch-response` `Bundle`.
    #[error("a batch answered a `{found}` Bundle, not a `batch-response`")]
    BatchType {
        /// The `Bundle.type` the answer declared.
        found: String,
    },
    /// A `Bundle` entry carries no `response.status`, or one that is not an
    /// HTTP status line.
    #[error("a batch response entry carries no usable response.status")]
    EntryStatus,
    /// A `Bundle` entry carries no resource, so the answer to its request is
    /// absent rather than negative.
    #[error("a batch response entry carries no resource")]
    EntryResource,
}

/// A call into the terminology server that did not reach an answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The request could not be built into a URL under the configured base.
    #[error("{base} cannot be a base URL for a FHIR service")]
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
    /// A request body could not be written as FHIR JSON.
    #[error("the {operation} request could not be written as FHIR JSON")]
    RequestBody {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// What the encoder reported.
        #[source]
        source: serde_json::Error,
    },
    /// The request never reached the server.
    #[error("the request to {url} could not be sent")]
    Transport {
        /// The URL that was called.
        url: Url,
        /// What the HTTP stack reported.
        #[source]
        source: reqwest::Error,
    },
    /// The server did not answer inside the configured timeout.
    #[error("the request to {url} timed out")]
    Timeout {
        /// The URL that was called.
        url: Url,
        /// What the HTTP stack reported.
        #[source]
        source: reqwest::Error,
    },
    /// The server answered `5xx`.
    #[error("the terminology server answered {upstream} for {url}")]
    ServerFailure {
        /// The URL that was called.
        url: Url,
        /// The upstream status and body.
        upstream: Box<UpstreamError>,
    },
    /// The server refused the credentials.
    #[error("the terminology server refused the credentials for {url}")]
    Unauthorized {
        /// The URL that was called.
        url: Url,
        /// The `WWW-Authenticate` challenge, when the server sent one.
        challenge: Option<String>,
        /// The upstream status and body.
        upstream: Box<UpstreamError>,
    },
    /// The server refused the operation.
    #[error("the terminology server refused {operation} with {upstream}")]
    Refused {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The URL that was called.
        url: Url,
        /// The upstream status and body, with any `tx-issue-type` coding.
        upstream: Box<UpstreamError>,
    },
    /// The response body could not be decoded.
    #[error("the body of the {operation} answer from {url} could not be decoded")]
    Body {
        /// The operation, as `Resource/$code`.
        operation: &'static str,
        /// The URL that was called.
        url: Url,
        /// What the decoder reported.
        #[source]
        source: Box<BodyError>,
    },
    /// The batch answered a different number of entries than were sent.
    #[error("the batch answered {answered} entries for {sent} requests")]
    BatchArity {
        /// How many requests were sent.
        sent: usize,
        /// How many entries came back.
        answered: usize,
    },
}

impl Error {
    /// Returns the variant's name as a short kebab-case label, for a log line
    /// that must name the failure class and never the URL or a body.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BaseUrl { .. } => "base-url",
            Self::ClientBuild { .. } => "client-build",
            Self::RequestBody { .. } => "request-body",
            Self::Transport { .. } => "transport",
            Self::Timeout { .. } => "timeout",
            Self::ServerFailure { .. } => "server-failure",
            Self::Unauthorized { .. } => "unauthorized",
            Self::Refused { .. } => "refused",
            Self::Body { .. } => "body",
            Self::BatchArity { .. } => "batch-arity",
        }
    }

    /// Returns whether a retry of the same request could succeed.
    ///
    /// Only a failure that did not reach the server, or one the server
    /// reported as its own (`5xx`), is transient. The three operations declare
    /// `affectsState: false`
    /// (`OperationDefinition-CodeSystem-lookup`,
    /// `OperationDefinition-ConceptMap-translate` and
    /// `OperationDefinition-ValueSet-validate-code`, R4B), so a repeat is the
    /// same read and the `POST` form is retried like the `GET` form
    /// (<https://hl7.org/fhir/R4/operations.html>).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport { .. } | Self::Timeout { .. } | Self::ServerFailure { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, TX_ISSUE_TYPE, TxIssue, UpstreamError};
    use crate::config::WireVersion;
    use http::StatusCode;

    fn outcome_body(tx_code: &str) -> String {
        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "code-invalid",
                "details": {
                    "coding": [{"system": TX_ISSUE_TYPE, "code": tx_code, "display": "Invalid code"}],
                    "text": "the code is not in the code system",
                },
                "diagnostics": "Unknown_Code_in_Version",
            }],
        })
        .to_string()
    }

    #[test]
    fn a_tx_issue_type_coding_is_surfaced() {
        let upstream = UpstreamError::new(
            WireVersion::R4,
            StatusCode::BAD_REQUEST,
            outcome_body("invalid-code"),
        );
        let outcome = upstream.outcome().expect("the body is an OperationOutcome");
        assert_eq!(1, outcome.issues.len());
        assert!(outcome.has_tx_issue("invalid-code"));
        assert_eq!(
            vec![&TxIssue {
                code: "invalid-code".to_owned(),
                display: Some("Invalid code".to_owned()),
            }],
            upstream.tx_issues().collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_r4b_outcome_decodes_the_same_way() {
        let upstream = UpstreamError::new(
            WireVersion::R4B,
            StatusCode::NOT_FOUND,
            outcome_body("not-found"),
        );
        assert!(
            upstream
                .outcome()
                .is_some_and(|outcome| outcome.has_tx_issue("not-found"))
        );
    }

    #[test]
    fn a_body_that_is_not_an_outcome_stays_raw() {
        let upstream =
            UpstreamError::new(WireVersion::R4, StatusCode::BAD_GATEWAY, "no".to_owned());
        assert!(upstream.outcome().is_none());
        assert_eq!("no", upstream.body());
        assert_eq!("502 Bad Gateway", upstream.to_string());
    }

    #[test]
    fn only_transport_timeout_and_server_errors_are_retried() {
        let url: url::Url = "http://tx.invalid/r4".parse().expect("a valid URL");
        let server = Error::ServerFailure {
            url: url.clone(),
            upstream: Box::new(UpstreamError::new(
                WireVersion::R4,
                StatusCode::SERVICE_UNAVAILABLE,
                String::new(),
            )),
        };
        assert!(server.is_retryable());
        let refused = Error::Refused {
            operation: "CodeSystem/$lookup",
            url,
            upstream: Box::new(UpstreamError::new(
                WireVersion::R4,
                StatusCode::BAD_REQUEST,
                String::new(),
            )),
        };
        assert!(!refused.is_retryable());
        assert_eq!("refused", refused.kind());
        assert!(
            !Error::BatchArity {
                sent: 2,
                answered: 1
            }
            .is_retryable()
        );
    }
}
