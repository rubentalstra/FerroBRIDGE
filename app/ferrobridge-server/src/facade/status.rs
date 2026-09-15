// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The one table from a CDR outcome to a facade answer.
//!
//! No specification governs the mapping: it is FerroBRIDGE's own design with
//! both sides cited per row (`docs/architecture.md` §4.6). ITS-REST 1.1.0
//! fixes what the CDR may answer (`ehr-codegen.openapi.yaml`, the status list
//! of each operation) and FHIR R4 fixes what a FHIR server may answer
//! (<https://hl7.org/fhir/R4/http.html>). Every row below is one [`Row`]
//! constant carrying both citations, and one wire test asserts each.
//!
//! The rows, in the order the architecture states them:
//!
//! - a CDR `422` (template validation) is a facade `422` with the
//!   `validationErrors` verbatim in `issue.diagnostics`;
//! - a CDR `412` is a `412` carrying the current `ETag`;
//! - a composition the CDR reports deleted (a `204` on the read) is a `410`;
//! - a CDR `401` propagates `WWW-Authenticate` and never becomes a `403`;
//! - a CDR `405` or `415` is a `500`, because the bridge chose a call the CDR
//!   does not offer;
//! - any CDR `5xx` is a `502` carrying the upstream status;
//! - a connect failure or a timeout is a `502`.

use ferrobridge_openehr::error::Error;
use ferrobridge_openehr::error::UpstreamError;
use http::StatusCode;

use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;

/// One row of the table, with the citation of each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The status the facade answers with.
    status: StatusCode,
    /// The `issue-type` code the `OperationOutcome` carries.
    issue: IssueType,
    /// Where ITS-REST 1.1.0 states the CDR side.
    openehr: &'static str,
    /// Where FHIR R4 states the facade side.
    fhir: &'static str,
}

impl Row {
    /// Returns the status the facade answers with.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the `issue-type` code of the authored outcome.
    #[must_use]
    pub const fn issue(&self) -> IssueType {
        self.issue
    }

    /// Returns the ITS-REST 1.1.0 citation of this row.
    #[must_use]
    pub const fn openehr_citation(&self) -> &'static str {
        self.openehr
    }

    /// Returns the FHIR R4 citation of this row.
    #[must_use]
    pub const fn fhir_citation(&self) -> &'static str {
        self.fhir
    }
}

/// A CDR `422`: the content is well formed and fails semantic validation.
pub const UNPROCESSABLE: Row = Row {
    status: StatusCode::UNPROCESSABLE_ENTITY,
    issue: IssueType::Invalid,
    openehr: "ehr-codegen.openapi.yaml, composition_create 422",
    fhir: "hl7.org/fhir/R4/http.html, create: 422 Unprocessable Entity",
};

/// A CDR `400`: the CDR could not parse or accept the request the bridge sent.
pub const BAD_REQUEST: Row = Row {
    status: StatusCode::INTERNAL_SERVER_ERROR,
    issue: IssueType::Exception,
    openehr: "ehr-codegen.openapi.yaml, composition_create 400",
    fhir: "hl7.org/fhir/R4/http.html, 500 Internal Server Error",
};

/// A CDR `412`: `If-Match` does not name the latest version.
pub const PRECONDITION_FAILED: Row = Row {
    status: StatusCode::PRECONDITION_FAILED,
    issue: IssueType::Conflict,
    openehr: "ehr-codegen.openapi.yaml, composition_update 412",
    fhir: "hl7.org/fhir/R4/http.html, update: 412 Precondition Failed",
};

/// A composition the CDR reports deleted.
///
/// A `GET` answers `204` "when the composition was deleted at the requested
/// time" (`ehr-codegen.openapi.yaml`, `composition_get`), which R4 renders as
/// `410 Gone` (<https://hl7.org/fhir/R4/http.html>, read).
pub const GONE: Row = Row {
    status: StatusCode::GONE,
    issue: IssueType::Deleted,
    openehr: "ehr-codegen.openapi.yaml, composition_get 204",
    fhir: "hl7.org/fhir/R4/http.html, read: 410 Gone",
};

/// A CDR `404`: no EHR, or no version of the resource the bridge addressed.
pub const NOT_FOUND: Row = Row {
    status: StatusCode::NOT_FOUND,
    issue: IssueType::NotFound,
    openehr: "ehr-codegen.openapi.yaml, composition_get 404",
    fhir: "hl7.org/fhir/R4/http.html, read: 404 Not Found",
};

/// A CDR `401`: the CDR refused the bridge's credentials.
///
/// The row answers `401` and never `403`: the CDR reported that it does not
/// know who is calling, which R4 states as `401 Not Authorized`
/// (<https://hl7.org/fhir/R4/http.html>). The `WWW-Authenticate` challenge is
/// propagated so a caller sees the scheme the CDR asked for.
pub const UNAUTHORIZED: Row = Row {
    status: StatusCode::UNAUTHORIZED,
    issue: IssueType::Login,
    openehr: "ITS-REST 1.1.0, Requests and responses, HTTP status codes: 401",
    fhir: "hl7.org/fhir/R4/http.html, 401 Not Authorized",
};

/// A status the CDR answered that its operation does not document.
///
/// A `405` or a `415` means the bridge chose a call the CDR does not offer, so
/// the fault is the bridge's and the answer is a `500`
/// (<https://hl7.org/fhir/R4/http.html>).
pub const UNDOCUMENTED: Row = Row {
    status: StatusCode::INTERNAL_SERVER_ERROR,
    issue: IssueType::Exception,
    openehr: "ITS-REST 1.1.0, Requests and responses, HTTP status codes",
    fhir: "hl7.org/fhir/R4/http.html, 500 Internal Server Error",
};

/// The CDR failed, or the call never reached it.
pub const UPSTREAM: Row = Row {
    status: StatusCode::BAD_GATEWAY,
    issue: IssueType::Transient,
    openehr: "ITS-REST 1.1.0, Requests and responses, HTTP status codes: 5xx",
    fhir: "hl7.org/fhir/R4/http.html, the server is a gateway to the CDR",
};

/// The identity store or another part of the bridge refused.
pub const INTERNAL: Row = Row {
    status: StatusCode::INTERNAL_SERVER_ERROR,
    issue: IssueType::Exception,
    openehr: "no CDR call was made",
    fhir: "hl7.org/fhir/R4/http.html, 500 Internal Server Error",
};

/// What the facade answers, with the row that decided it.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The row that decided the status.
    row: Row,
    /// The issue the `OperationOutcome` carries.
    issue: Issue,
    /// The `ETag` the answer carries, when the row states one.
    entity_tag: Option<String>,
    /// The `WWW-Authenticate` challenge the answer propagates, when the CDR
    /// sent one.
    challenge: Option<String>,
}

impl Answer {
    /// Returns the answer `row` decides, diagnosing `detail`.
    #[must_use]
    pub fn new(row: Row, detail: String) -> Self {
        Self {
            row,
            issue: Issue::error(row.issue).diagnosing(detail),
            entity_tag: None,
            challenge: None,
        }
    }

    /// Returns this answer carrying `tag` as its `ETag`.
    #[must_use]
    pub fn with_entity_tag(mut self, tag: impl Into<String>) -> Self {
        self.entity_tag = Some(tag.into());
        self
    }

    /// Returns the row that decided the status.
    #[must_use]
    pub const fn row(&self) -> Row {
        self.row
    }

    /// Returns the status the facade answers with.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.row.status
    }

    /// Returns the issue the `OperationOutcome` carries.
    #[must_use]
    pub const fn issue(&self) -> &Issue {
        &self.issue
    }

    /// Returns the `ETag` the answer carries.
    #[must_use]
    pub fn entity_tag(&self) -> Option<&str> {
        self.entity_tag.as_deref()
    }

    /// Returns the `WWW-Authenticate` challenge the answer propagates.
    #[must_use]
    pub fn challenge(&self) -> Option<&str> {
        self.challenge.as_deref()
    }
}

/// Returns the diagnostics text of one upstream answer.
///
/// The openEHR error body travels verbatim: the `message` and every
/// `validationErrors` entry the CDR sent, and the raw body when the CDR sent
/// something that is not an error body. It never reaches the wire as its own
/// document (`docs/architecture.md` §4.6).
#[must_use]
pub fn diagnostics(upstream: &UpstreamError) -> String {
    let mut text = format!("the openEHR service answered {}", upstream.status());
    match upstream.error() {
        Some(error) => {
            if let Some(message) = error.message.as_deref() {
                text.push_str(": ");
                text.push_str(message);
            }
            for entry in &error.validation_errors {
                text.push_str("; ");
                text.push_str(entry);
            }
        }
        None if !upstream.body().is_empty() => {
            text.push_str(": ");
            text.push_str(upstream.body());
        }
        None => {}
    }
    text
}

/// Returns the answer one client-level refusal maps to.
///
/// The client returns [`Error`] only for a call that reached no documented
/// answer, so every variant here is a bridge-side or transport-side fault and
/// none of them is flattened into an empty value.
#[must_use]
pub fn of_client_error(error: &Error) -> Answer {
    match *error {
        Error::Unauthorized {
            ref challenge,
            ref upstream,
            ..
        } => Answer {
            row: UNAUTHORIZED,
            issue: Issue::error(UNAUTHORIZED.issue).diagnosing(diagnostics(upstream)),
            entity_tag: None,
            challenge: challenge.clone(),
        },
        Error::ServiceFailure { ref upstream, .. } => Answer::new(UPSTREAM, diagnostics(upstream)),
        Error::Transport { .. } | Error::Timeout { .. } => Answer::new(
            UPSTREAM,
            String::from("the openEHR service could not be reached"),
        ),
        Error::UndocumentedStatus { ref upstream, .. } => {
            Answer::new(UNDOCUMENTED, diagnostics(upstream))
        }
        _ => Answer::new(
            INTERNAL,
            format!("the openEHR client refused this call ({})", error.kind()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BAD_REQUEST, GONE, NOT_FOUND, PRECONDITION_FAILED, UNAUTHORIZED, UNDOCUMENTED,
        UNPROCESSABLE, UPSTREAM, diagnostics, of_client_error,
    };
    use ferrobridge_openehr::error::{Error, UpstreamError};
    use http::StatusCode;

    #[test]
    fn every_row_states_both_sides() {
        for row in [
            UNPROCESSABLE,
            BAD_REQUEST,
            PRECONDITION_FAILED,
            GONE,
            NOT_FOUND,
            UNAUTHORIZED,
            UNDOCUMENTED,
            UPSTREAM,
        ] {
            assert!(
                !row.openehr_citation().is_empty() && !row.fhir_citation().is_empty(),
                "a row of the status table carries no citation"
            );
        }
    }

    #[test]
    fn the_table_maps_each_documented_cdr_answer() {
        assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, UNPROCESSABLE.status());
        assert_eq!(
            StatusCode::PRECONDITION_FAILED,
            PRECONDITION_FAILED.status()
        );
        assert_eq!(StatusCode::GONE, GONE.status());
        assert_eq!(StatusCode::NOT_FOUND, NOT_FOUND.status());
        assert_eq!(StatusCode::BAD_GATEWAY, UPSTREAM.status());
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, UNDOCUMENTED.status());
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, BAD_REQUEST.status());
    }

    #[test]
    fn a_401_stays_a_401_and_propagates_its_challenge() {
        let error = Error::Unauthorized {
            url: "http://cdr.invalid/ehr".parse().expect("a URL"),
            challenge: Some(String::from("Bearer realm=\"cdr\"")),
            upstream: Box::new(UpstreamError::new(StatusCode::UNAUTHORIZED, String::new())),
        };
        let answer = of_client_error(&error);
        assert_eq!(StatusCode::UNAUTHORIZED, answer.status());
        assert_ne!(StatusCode::FORBIDDEN, answer.status());
        assert_eq!(Some("Bearer realm=\"cdr\""), answer.challenge());
    }

    #[test]
    fn a_5xx_becomes_a_502_carrying_the_upstream_status() {
        let error = Error::ServiceFailure {
            url: "http://cdr.invalid/ehr".parse().expect("a URL"),
            upstream: Box::new(UpstreamError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                String::new(),
            )),
        };
        let answer = of_client_error(&error);
        assert_eq!(StatusCode::BAD_GATEWAY, answer.status());
        let built = answer.issue().build();
        assert!(
            built
                .diagnostics
                .and_then(|text| text.value)
                .is_some_and(|text| text.contains("503")),
            "the 502 does not carry the upstream status"
        );
    }

    #[test]
    fn the_validation_errors_travel_verbatim() {
        let upstream = UpstreamError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"message":"the composition is invalid","validationErrors":["/content[0] is required"]}"#
                .to_owned(),
        );
        let text = diagnostics(&upstream);
        assert!(text.contains("the composition is invalid"), "{text}");
        assert!(text.contains("/content[0] is required"), "{text}");
    }

    #[test]
    fn a_body_that_is_no_error_document_still_reaches_the_diagnostics() {
        let upstream = UpstreamError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            String::from("a plain-text refusal"),
        );
        assert!(diagnostics(&upstream).contains("a plain-text refusal"));
    }
}
