// SPDX-FileCopyrightText: Vernum Projecten B.V.
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

use http::StatusCode;
use openehr_its::rest::client::ClientError;

use crate::cdr::error::CdrError;
use crate::cdr::error::Upstream;
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
pub fn diagnostics(upstream: &Upstream) -> String {
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

/// Returns the diagnostics of a refusal the generated client read, from the
/// answer the CDR sent when it was kept, and from the status and the `Error`
/// message the refusal carries otherwise.
fn refusal_diagnostics(
    upstream: Option<&Upstream>,
    status: StatusCode,
    detail: Option<&str>,
) -> String {
    match upstream {
        Some(upstream) => diagnostics(upstream),
        None => match detail {
            Some(message) => format!("the openEHR service answered {status}: {message}"),
            None => format!("the openEHR service answered {status}"),
        },
    }
}

/// Returns the answer one CDR call that reached no usable documented answer
/// maps to.
///
/// Every variant here is a bridge-side, transport-side or CDR-side fault the
/// generated client or the bridge reported, and none of them is flattened into
/// an empty value.
#[must_use]
pub fn of_client_error(error: &CdrError) -> Answer {
    match *error {
        CdrError::Client {
            ref source,
            ref upstream,
        } => of_generated_error(source, upstream.as_deref()),
        _ => Answer::new(
            INTERNAL,
            format!("the openEHR client refused this call ({})", error.kind()),
        ),
    }
}

/// Returns the answer one refusal of the generated client maps to, `upstream`
/// being the answer the CDR sent when the call reached it.
fn of_generated_error(error: &ClientError, upstream: Option<&Upstream>) -> Answer {
    match *error {
        ClientError::Unauthorized {
            ref challenge,
            ref detail,
            ..
        } => Answer {
            row: UNAUTHORIZED,
            issue: Issue::error(UNAUTHORIZED.issue).diagnosing(refusal_diagnostics(
                upstream,
                StatusCode::UNAUTHORIZED,
                detail.as_deref(),
            )),
            entity_tag: None,
            challenge: challenge.clone(),
        },
        ClientError::ServiceFailure {
            status, ref detail, ..
        } => Answer::new(
            UPSTREAM,
            refusal_diagnostics(upstream, status, detail.as_deref()),
        ),
        ClientError::Transport { .. } => Answer::new(
            UPSTREAM,
            String::from("the openEHR service could not be reached"),
        ),
        ClientError::UndocumentedStatus {
            status, ref detail, ..
        } => Answer::new(
            UNDOCUMENTED,
            refusal_diagnostics(upstream, status, detail.as_deref()),
        ),
        // NOTE: ITS-REST 1.1.0 §Requests and responses/HTTP status codes lists
        // `403` for every operation and no operation documents it, so the row is
        // the one every status outside an operation's set takes.
        ClientError::Forbidden { ref detail, .. } => Answer::new(
            UNDOCUMENTED,
            refusal_diagnostics(upstream, StatusCode::FORBIDDEN, detail.as_deref()),
        ),
        _ => Answer::new(
            INTERNAL,
            format!(
                "the openEHR client refused this call ({})",
                crate::cdr::error::client_kind(error)
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BAD_REQUEST, GONE, NOT_FOUND, PRECONDITION_FAILED, UNAUTHORIZED, UNDOCUMENTED,
        UNPROCESSABLE, UPSTREAM, diagnostics, of_client_error,
    };
    use crate::cdr::error::{CdrError, Upstream};
    use http::{HeaderMap, Method, StatusCode};
    use openehr_its::rest::client::ClientError;

    fn upstream(status: StatusCode, body: &str) -> Upstream {
        Upstream::new(status, HeaderMap::new(), body.to_owned())
    }

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
        let error = CdrError::client(
            ClientError::Unauthorized {
                method: Method::GET,
                path: String::from("/ehr"),
                challenge: Some(String::from("Bearer realm=\"cdr\"")),
                detail: None,
            },
            Some(upstream(StatusCode::UNAUTHORIZED, "")),
        );
        let answer = of_client_error(&error);
        assert_eq!(StatusCode::UNAUTHORIZED, answer.status());
        assert_ne!(StatusCode::FORBIDDEN, answer.status());
        assert_eq!(Some("Bearer realm=\"cdr\""), answer.challenge());
    }

    #[test]
    fn a_5xx_becomes_a_502_carrying_the_upstream_status() {
        let error = CdrError::client(
            ClientError::ServiceFailure {
                method: Method::GET,
                path: String::from("/ehr"),
                status: StatusCode::SERVICE_UNAVAILABLE,
                detail: None,
            },
            None,
        );
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
    fn a_403_is_a_status_outside_the_documented_set() {
        let error = CdrError::client(
            ClientError::Forbidden {
                method: Method::POST,
                path: String::from("/ehr"),
                detail: Some(String::from("no write grant")),
            },
            None,
        );
        let answer = of_client_error(&error);
        assert_eq!(UNDOCUMENTED, answer.row());
        let built = answer.issue().build();
        assert!(
            built
                .diagnostics
                .and_then(|text| text.value)
                .is_some_and(|text| text.contains("403") && text.contains("no write grant")),
            "the 500 does not carry the upstream status and message"
        );
    }

    #[test]
    fn the_validation_errors_travel_verbatim() {
        let upstream = upstream(
            StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"message":"the composition is invalid","validationErrors":["/content[0] is required"]}"#,
        );
        let text = diagnostics(&upstream);
        assert!(text.contains("the composition is invalid"), "{text}");
        assert!(text.contains("/content[0] is required"), "{text}");
    }

    #[test]
    fn a_body_that_is_no_error_document_still_reaches_the_diagnostics() {
        let upstream = upstream(StatusCode::UNPROCESSABLE_ENTITY, "a plain-text refusal");
        assert!(diagnostics(&upstream).contains("a plain-text refusal"));
    }

    #[test]
    fn an_error_body_member_outside_the_openehr_shape_stays_out_of_the_diagnostics() {
        let upstream = upstream(
            StatusCode::CONFLICT,
            r#"{"message":"an EHR exists","trace":"ferrobridge-stack-trace-0001"}"#,
        );
        let text = diagnostics(&upstream);
        assert!(text.contains("an EHR exists"), "{text}");
        assert!(!text.contains("ferrobridge-stack-trace-0001"), "{text}");
    }
}
