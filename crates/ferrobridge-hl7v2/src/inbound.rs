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
//! `AE` with an `ERR` per refusal. Anything else is handed to the caller to
//! map, who answers `AA` or `AE` once the message is committed or refused.
//! A message naming its sender in none of MSH-3, MSH-24 and MSH-4 is mapped
//! too: the guide's MSH-24 row writes a `data-absent-reason` into
//! `MessageHeader.source.endpoint` for it (`segment-msh-to-messageheader`).

use hl7v2_types::model::Structure;

use crate::ack::{self, Code, Error, Header, Stamp};
use crate::decode::{self, Charset, DecodeError};
use crate::parse::structure::StructureError;
use crate::parse::{self, ErrorCode, Message, Parsed};

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
/// lexed message ([`parse::structure::structure_for`] selects it from the definitions).
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
    let lexed = match parse::lex::lex(&decoded.text, decoded.charset) {
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
    let parsed = parse::grouping::group(lexed, structure);
    if !parsed.refusals().is_empty() {
        let errors: Vec<Error> = parsed.refusals().iter().map(Error::from).collect();
        return answer(Code::Error, &errors);
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

/// An `ERR` with no location.
fn error_of(code: ErrorCode, text: &str) -> Error {
    Error {
        location: None,
        code,
        text: String::from(text),
    }
}
