// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The acknowledgment a mapped message's outcome owes.
//!
//! Original mode answers `AA` when the receiver processed the message, `AE`
//! with "an error response, providing error information", and `AR` when it
//! failed to process the message "for reasons unrelated to content" (HL7
//! v2.5.1 chapter 2 §2.9.2.2; the codes are HL7 table 0008). A message the
//! ingest committed, or one whose every entry an earlier delivery committed,
//! is `AA`. A refusal of the message's content (no program, a second subject,
//! an entry that does not map, a condition that stops the mapper) is `AE`
//! with one `ERR` per refused entry. A refusal that says nothing about the
//! content (the CDR or the terminology server unreachable, refusing the
//! bridge's credentials, or failing) is `AR`, so the sender resends it later.

use std::collections::BTreeMap;

use ferrobridge_hl7v2::ack::{Code, Error};
use ferrobridge_hl7v2::map::MapError;
use ferrobridge_hl7v2::parse::ErrorCode;
use http::StatusCode;

use crate::facade::ingest::{Ingested, Refused};
use crate::facade::outcome::Issue;

/// How one parsed message settled.
#[derive(Debug)]
pub enum Settled {
    /// The ingest committed the message, or found it committed.
    Ingested(Box<Ingested>),
    /// The ingest refused the message's Bundle.
    Refused(Refused),
    /// The v2-to-FHIR run refused the message.
    Unmapped(MapError),
    /// MSH-10 is empty, so the message cannot key the identity map.
    Unkeyed,
    /// MSH-4 names a sending facility the face does not accept.
    Sender,
}

/// What one settled message produced, for the log.
#[derive(Debug, Default)]
pub struct Tally {
    /// The run's counted outcomes by kind, when the run completed.
    pub outcomes: BTreeMap<&'static str, usize>,
    /// The entries the ingest committed.
    pub committed: usize,
    /// The entries the ingest skipped.
    pub skipped: usize,
}

/// Returns the acknowledgment code and the `ERR` segments `settled` owes.
#[must_use]
pub fn acknowledgment(settled: &Settled) -> (Code, Vec<Error>) {
    match settled {
        Settled::Ingested(_) => (Code::Accept, Vec::new()),
        Settled::Refused(refused) => of_refusal(refused),
        Settled::Unmapped(error) => of_map_error(error),
        Settled::Sender => (
            Code::Reject,
            vec![Error {
                location: Some(ferrobridge_hl7v2::parse::Location::segment("MSH", 1).with_field(4)),
                code: ErrorCode::Application,
                text: String::from("MSH-4 names a sending facility this receiver does not accept"),
            }],
        ),
        Settled::Unkeyed => (
            Code::Error,
            vec![Error {
                location: Some(
                    ferrobridge_hl7v2::parse::Location::segment("MSH", 1).with_field(10),
                ),
                code: ErrorCode::RequiredFieldMissing,
                text: String::from(
                    "MSH-10 is empty, so the message cannot be recognised when resent",
                ),
            }],
        ),
    }
}

/// Returns the answer to a refusal of the ingest.
///
/// A status no content change can clear is `AR`: a `5xx` (the CDR failed or
/// could not be reached), a `401`, `403` or `407` (the CDR refused the
/// bridge's own credentials), a `408` and a `429`. Every other refusal is
/// about the message and is `AE` (no specification governs the split: our
/// own design over §2.9.2.2's two definitions).
fn of_refusal(refused: &Refused) -> (Code, Vec<Error>) {
    let status = refused.status();
    let unrelated = status.is_server_error()
        || [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::PROXY_AUTHENTICATION_REQUIRED,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
        ]
        .contains(&status);
    let code = if unrelated { Code::Reject } else { Code::Error };
    let errors = refused.issues().iter().map(error_of).collect();
    (code, errors)
}

/// Returns one `ERR` for one issue of a refusal: table 0357 `207`, with the
/// entry it names and its diagnostics as the text.
fn error_of(issue: &Issue) -> Error {
    let detail = issue.diagnostics().unwrap_or(issue.code().as_str());
    let text = match issue.locations() {
        [] => String::from(detail),
        places => format!("{}: {detail}", places.join(", ")),
    };
    Error {
        location: None,
        code: ErrorCode::Application,
        text,
    }
}

/// Returns the answer to a refusal of the v2-to-FHIR run.
///
/// A structure the guide maps no message of is an unsupported message type
/// (table 0357 `200`), and a translation the terminology server refused or
/// never answered says nothing about the message, so both are `AR`. A
/// condition that stops the mapper and a message that yields no
/// `MessageHeader` are about its content, so both are `AE`.
fn of_map_error(error: &MapError) -> (Code, Vec<Error>) {
    let (code, error_code, location) = match error {
        MapError::NoMessageMap { .. } => (Code::Reject, ErrorCode::UnsupportedMessageType, None),
        MapError::Terminology { .. } => (Code::Reject, ErrorCode::Application, None),
        MapError::Stopped { at, .. } => (
            Code::Error,
            ErrorCode::RequiredFieldMissing,
            Some(at.as_ref().clone()),
        ),
        MapError::NoMessageHeader { .. } => (Code::Error, ErrorCode::Application, None),
    };
    (
        code,
        vec![Error {
            location,
            code: error_code,
            text: error.to_string(),
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::{Settled, acknowledgment};
    use crate::facade::ingest::Refused;
    use crate::facade::outcome::{Issue, IssueType};
    use ferrobridge_hl7v2::ack::Code;
    use ferrobridge_hl7v2::map::MapError;
    use ferrobridge_hl7v2::parse::ErrorCode;
    use http::StatusCode;

    fn refused(status: StatusCode) -> Settled {
        Settled::Refused(Refused::of(
            status,
            vec![
                Issue::error(IssueType::Required)
                    .diagnosing("the Bundle carries no entry this server maps"),
                Issue::error(IssueType::NotSupported)
                    .diagnosing("no loaded mapping answers for Observation")
                    .at("urn:uuid:0000-observation"),
            ],
        ))
    }

    #[test]
    fn a_refusal_of_the_content_is_ae_with_one_err_per_issue() {
        let (code, errors) = acknowledgment(&refused(StatusCode::UNPROCESSABLE_ENTITY));
        assert_eq!(Code::Error, code);
        let texts: Vec<&str> = errors.iter().map(|error| error.text.as_str()).collect();
        assert_eq!(
            vec![
                "the Bundle carries no entry this server maps",
                "urn:uuid:0000-observation: no loaded mapping answers for Observation",
            ],
            texts
        );
        assert!(
            errors
                .iter()
                .all(|error| error.code == ErrorCode::Application && error.location.is_none())
        );
    }

    #[test]
    fn a_conflict_with_what_the_map_holds_is_ae() {
        assert_eq!(
            Code::Error,
            acknowledgment(&refused(StatusCode::CONFLICT)).0
        );
    }

    #[test]
    fn an_upstream_that_failed_or_refused_the_bridge_is_ar() {
        for status in [
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
        ] {
            assert_eq!(Code::Reject, acknowledgment(&refused(status)).0, "{status}");
        }
    }

    #[test]
    fn a_structure_the_guide_does_not_map_is_ar_unsupported_message_type() {
        let (code, errors) = acknowledgment(&Settled::Unmapped(MapError::NoMessageMap {
            structure: String::from("ZZZ_Z01"),
        }));
        assert_eq!(Code::Reject, code);
        assert_eq!(
            vec![ErrorCode::UnsupportedMessageType],
            errors.iter().map(|error| error.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_message_with_no_message_header_is_ae() {
        let (code, _) = acknowledgment(&Settled::Unmapped(MapError::NoMessageHeader {
            dropped: None,
        }));
        assert_eq!(Code::Error, code);
    }

    #[test]
    fn a_sender_not_accepted_is_ar_at_msh_4() {
        let (code, errors) = acknowledgment(&Settled::Sender);
        assert_eq!(Code::Reject, code);
        let location = errors
            .first()
            .and_then(|error| error.location.as_ref())
            .expect("the field is named");
        assert_eq!("MSH^1^4", location.erl('^'));
    }

    #[test]
    fn an_empty_msh_10_is_ae_naming_the_field() {
        let (code, errors) = acknowledgment(&Settled::Unkeyed);
        assert_eq!(Code::Error, code);
        let location = errors
            .first()
            .and_then(|error| error.location.as_ref())
            .expect("the field is named");
        assert_eq!(
            ("MSH", Some(10)),
            (location.segment.as_str(), location.field)
        );
    }
}
