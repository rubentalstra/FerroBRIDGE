// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Receiving one message: decode, parse, group, and the answer each failure
//! owes.
//!
//! A message that cannot be read as HL7 v2 at all (no MSH, delimiters that do
//! not declare themselves, a character set outside the one declared, a
//! structure the definitions do not carry, a version outside 2.x) is refused
//! with `AR`, since the refusal is "for reasons unrelated to content" (HL7
//! v2.5.1 chapter 2 §2.9.2.2). A message that parses but carries a refusal
//! (a missing required field or segment, an undecoded escape) is answered
//! `AE` with an `ERR` per refusal. A message with none that names its sender
//! in none of MSH-3, MSH-24 and MSH-4 is refused with `AR`, since no
//! `MessageHeader.source` can be written for it. Anything else is handed to
//! the caller to map, who answers `AA` or `AE` once the message is committed
//! or refused.

use hl7v2_types::model::Structure;

use crate::ack::{self, Code, Error, Header, Stamp};
use crate::decode::{self, Charset, DecodeError};
use crate::parse::{self, ErrorCode, Field, Location, Message, Parsed, StructureError};

/// A parsed message and what its answer is built from.
#[derive(Debug, Clone)]
pub struct Inbound {
    parsed: Parsed,
    header: Header,
    charset: Charset,
}

impl Inbound {
    /// Returns the parsed message.
    #[must_use]
    pub const fn parsed(&self) -> &Parsed {
        &self.parsed
    }

    /// Returns the header the answer echoes.
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// Builds the answer, encoded in the character set the message came in.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Undeclared`] when an echoed value has no byte in
    /// that set.
    pub fn answer(
        &self,
        code: Code,
        errors: &[Error],
        stamp: Stamp<'_>,
    ) -> Result<Vec<u8>, DecodeError> {
        ack::encoded(&self.header, code, errors, stamp, self.charset)
    }
}

/// What receiving one message produced.
#[derive(Debug, Clone)]
pub enum Received {
    /// The message parsed with no refusal and waits to be mapped.
    Parsed(Box<Inbound>),
    /// The message is answered here, `AR` or `AE`, and is not mapped.
    Answered {
        /// The acknowledgment code.
        code: Code,
        /// The acknowledgment's bytes.
        reply: Vec<u8>,
    },
    /// No header can be read, so no acknowledgment can be addressed.
    Unanswerable,
}

/// Receives one message's bytes.
///
/// `default` is the character set the connection agreed on for a message
/// whose MSH-18 is empty, and `select` picks the message structure from the
/// lexed message ([`parse::structure_for`] selects it from the definitions).
#[must_use]
pub fn receive(
    message: &[u8],
    default: Charset,
    select: impl Fn(&Message) -> Result<&'static Structure, StructureError>,
    stamp: Stamp<'_>,
) -> Received {
    let decoded = match decode::decode(message, default) {
        Ok(decoded) => decoded,
        Err(error) => return reject_bytes(message, &error.to_string(), stamp),
    };
    let Some(header) = Header::from_text(&decoded.text) else {
        return Received::Unanswerable;
    };
    let answer = |code: Code, errors: &[Error]| match ack::encoded(
        &header,
        code,
        errors,
        stamp,
        decoded.charset,
    ) {
        Ok(reply) => Received::Answered { code, reply },
        Err(_) => Received::Unanswerable,
    };
    let lexed = match parse::lex(&decoded.text, decoded.charset) {
        Ok(lexed) => lexed,
        Err(error) => {
            return answer(
                Code::Reject,
                &[error_of(ErrorCode::Application, &error.to_string())],
            );
        }
    };
    let version = lexed.message.version().unwrap_or_default();
    if !version.starts_with("2.") {
        return answer(
            Code::Reject,
            &[error_of(
                ErrorCode::UnsupportedVersion,
                "MSH-12 names no HL7 version 2.x",
            )],
        );
    }
    let structure = match select(&lexed.message) {
        Ok(structure) => structure,
        Err(error) => {
            return answer(
                Code::Reject,
                &[error_of(
                    ErrorCode::UnsupportedMessageType,
                    &error.to_string(),
                )],
            );
        }
    };
    let parsed = parse::group(lexed, structure);
    if !parsed.refusals().is_empty() {
        let errors: Vec<Error> = parsed.refusals().iter().map(Error::from).collect();
        return answer(Code::Error, &errors);
    }
    if !identifies_sender(parsed.message()) {
        return answer(
            Code::Reject,
            &[Error {
                location: Some(Location::segment("MSH", 1).with_field(3)),
                code: ErrorCode::RequiredFieldMissing,
                text: String::from(
                    "MSH-3, MSH-24 and MSH-4 name no sending application, network address or facility",
                ),
            }],
        );
    }
    Received::Parsed(Box::new(Inbound {
        parsed,
        header,
        charset: decoded.charset,
    }))
}

/// The `AR` answer to bytes that could not be decoded, addressed from the
/// header read as ASCII.
fn reject_bytes(message: &[u8], detail: &str, stamp: Stamp<'_>) -> Received {
    match ack::reject_frame(message, detail, stamp) {
        Some(reply) => Received::Answered {
            code: Code::Reject,
            reply,
        },
        None => Received::Unanswerable,
    }
}

/// Whether a message to be mapped names its sender in MSH-3 or MSH-24, the
/// fields the guide's MSH map writes `MessageHeader.source` from, or in
/// MSH-4, the sending facility the map falls back to.
///
/// The guide leaves a message valuing neither MSH-3 nor MSH-24 to the
/// implementer (`segment-msh-to-messageheader`, the MSH-3 row's comment), and
/// FHIR R4 requires `MessageHeader.source`
/// (<https://hl7.org/fhir/R4/messageheader.html>). No specification governs
/// the answer: our own design takes MSH-4 and refuses a message valuing none
/// of the three with `AR`.
/// An acknowledgment is never mapped, so it owes no `MessageHeader`.
fn identifies_sender(message: &Message) -> bool {
    if message.message_type(1) == Some("ACK") {
        return true;
    }
    message.header().is_some_and(|header| {
        [3, 24, 4]
            .into_iter()
            .any(|position| header.field(position).is_some_and(Field::is_valued))
    })
}

/// An `ERR` with no location.
fn error_of(code: ErrorCode, text: &str) -> Error {
    Error {
        location: None,
        code,
        text: String::from(text),
    }
}
