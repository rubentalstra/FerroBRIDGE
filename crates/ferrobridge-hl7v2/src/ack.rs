// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The original-mode application acknowledgment.
//!
//! The receiver answers `AA` when it processed the message, `AE` when it
//! sends "an error response, providing error information", and `AR` when it
//! failed to process the message "for reasons unrelated to content" (HL7
//! v2.5.1 chapter 2 §2.9.2.2; the codes are HL7 table 0008,
//! `CodeSystem/v2-0008` in the vendored `hl7.terminology` package). MSA-2
//! carries MSH-10 of the message acknowledged (§2.9.2.2), and each refusal
//! is one `ERR` segment with its location in ERR-2, its table 0357 code in
//! ERR-3 and the severity `E` in ERR-4.
//!
//! The acknowledgment's header swaps the sending and receiving application
//! and facility of the message (MSH-3 to MSH-6), keeps its delimiters,
//! processing id, version and character set, and names the message type
//! `ACK` with the trigger event of the message. The values echoed are taken
//! as written, escapes included, so they travel unchanged.

use crate::decode::{Charset, DecodeError};
use crate::parse::{ErrorCode, Location, Refusal};

/// An acknowledgment code of HL7 table 0008, original mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// `AA`, application accept.
    Accept,
    /// `AE`, application error.
    Error,
    /// `AR`, application reject.
    Reject,
}

impl Code {
    /// Returns the table 0008 code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "AA",
            Self::Error => "AE",
            Self::Reject => "AR",
        }
    }
}

/// The header fields of the message acknowledged, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    field: char,
    encoding: String,
    component: char,
    fields: Vec<String>,
}

impl Header {
    /// Reads the header from the message text's first segment.
    ///
    /// `None` when the text opens with no MSH segment or its MSH-2 holds
    /// fewer than four encoding characters.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        let line = text.split('\r').next()?;
        let rest = line.strip_prefix("MSH")?;
        let field = rest.chars().next()?;
        let mut pieces = rest.get(field.len_utf8()..)?.split(field);
        let encoding = String::from(pieces.next()?);
        let mut encoding_characters = encoding.chars();
        let component = encoding_characters.next()?;
        if encoding_characters.count() < 3 {
            return None;
        }
        Some(Self {
            field,
            encoding,
            component,
            fields: pieces.map(String::from).collect(),
        })
    }

    /// Reads the header from undecoded bytes, for a reject of a message that
    /// could not be decoded or framed.
    ///
    /// `None` when the first segment holds a byte outside 7-bit ASCII, since
    /// no character set is known to read it in.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let end = bytes
            .iter()
            .position(|byte| *byte == b'\r')
            .unwrap_or(bytes.len());
        let line = bytes.get(..end)?;
        if !line.is_ascii() {
            return None;
        }
        Self::from_text(core::str::from_utf8(line).ok()?)
    }

    /// Returns the field at `position` as written, from MSH-3; `None` for
    /// MSH-1 and MSH-2 and for an absent field.
    fn field(&self, position: usize) -> Option<&str> {
        self.fields
            .get(position.checked_sub(3)?)
            .map(String::as_str)
    }

    /// Returns MSH-10 as written.
    #[must_use]
    pub fn control_id(&self) -> Option<&str> {
        self.field(10).filter(|value| !value.is_empty())
    }

    /// Returns the message code and trigger event of MSH-9 as written, joined
    /// by `^` (`ORU^R01`); `None` when MSH-9.1 is empty.
    #[must_use]
    pub fn type_code(&self) -> Option<String> {
        let code = self.message_type(1).filter(|code| !code.is_empty())?;
        Some(
            match self.message_type(2).filter(|event| !event.is_empty()) {
                Some(event) => format!("{code}^{event}"),
                None => String::from(code),
            },
        )
    }

    /// Returns the component at `position` of MSH-9 as written, from 1.
    fn message_type(&self, position: usize) -> Option<&str> {
        self.field(9)?
            .split(self.component)
            .nth(position.checked_sub(1)?)
    }
}

/// What the acknowledgment's own header carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp<'a> {
    /// MSH-10 of the acknowledgment.
    pub control_id: &'a str,
    /// MSH-7 of the acknowledgment, a v2 `DTM`.
    pub timestamp: &'a str,
}

/// One `ERR` segment of an acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// ERR-2, the location, when the error has one.
    pub location: Option<Location>,
    /// ERR-3, the table 0357 code.
    pub code: ErrorCode,
    /// ERR-8, the user message, without message content.
    pub text: String,
}

impl From<&Refusal> for Error {
    fn from(refusal: &Refusal) -> Self {
        Self {
            location: Some(refusal.location.clone())
                .filter(|location| !location.segment.is_empty()),
            code: refusal.code,
            text: refusal.detail.clone(),
        }
    }
}

/// Renders the acknowledgment of the message `header` names.
#[must_use]
pub fn render(header: &Header, code: Code, errors: &[Error], stamp: Stamp<'_>) -> String {
    let field = header.field.to_string();
    let component = header.component.to_string();
    let escape_character = header.encoding.chars().nth(2);
    let escape = |text: &str| escape_with(header, escape_character, text);
    let echoed = |position: usize| header.field(position).unwrap_or_default();
    let event = header.message_type(2).unwrap_or_default();
    let mut msh = vec![
        String::from("MSH"),
        header.encoding.clone(),
        String::from(echoed(5)),
        String::from(echoed(6)),
        String::from(echoed(3)),
        String::from(echoed(4)),
        escape(stamp.timestamp),
        String::new(),
        ["ACK", event, "ACK"].join(&component),
        escape(stamp.control_id),
        String::from(echoed(11)),
        String::from(echoed(12)),
    ];
    if let Some(charset) = header.field(18).filter(|value| !value.is_empty()) {
        msh.resize(17, String::new());
        msh.push(String::from(charset));
    }
    let mut segments = vec![
        msh.join(&field),
        [
            "MSA",
            code.as_str(),
            header.control_id().unwrap_or_default(),
        ]
        .join(&field),
    ];
    for error in errors {
        let location = error
            .location
            .as_ref()
            .map(|location| location.erl(header.component))
            .unwrap_or_default();
        let condition = [error.code.code(), error.code.display(), "HL70357"].join(&component);
        segments.push(
            [
                "ERR",
                "",
                &location,
                &condition,
                "E",
                "",
                "",
                "",
                &escape(&error.text),
            ]
            .join(&field),
        );
    }
    let mut text = segments.join("\r");
    text.push('\r');
    text
}

/// Renders and encodes the acknowledgment in the message's character set.
///
/// # Errors
///
/// Returns [`DecodeError::Undeclared`] when an echoed value has no byte in
/// `charset`.
pub fn encoded(
    header: &Header,
    code: Code,
    errors: &[Error],
    stamp: Stamp<'_>,
    charset: Charset,
) -> Result<Vec<u8>, DecodeError> {
    crate::decode::encode(&render(header, code, errors, stamp), charset)
}

/// Builds the `AR` answer to a malformed frame from the bytes read before the
/// refusal, `None` when no header can be read from them.
#[must_use]
pub fn reject_frame(partial: &[u8], detail: &str, stamp: Stamp<'_>) -> Option<Vec<u8>> {
    let header = Header::from_bytes(partial)?;
    let error = Error {
        location: None,
        code: ErrorCode::Application,
        text: String::from(detail),
    };
    Some(render(&header, Code::Reject, &[error], stamp).into_bytes())
}

/// Escapes our own text with the message's delimiters (chapter 2 §2.7.1).
fn escape_with(header: &Header, escape: Option<char>, text: &str) -> String {
    let Some(escape) = escape else {
        return String::from(text);
    };
    let mut specials: Vec<(char, char)> = vec![(header.field, 'F')];
    let mut encoding = header.encoding.chars();
    for code in ['S', 'R', 'E', 'T'] {
        if let Some(character) = encoding.next() {
            specials.push((character, code));
        }
    }
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match specials.iter().find(|(special, _)| *special == character) {
            Some((_, code)) => {
                out.push(escape);
                out.push(*code);
                out.push(escape);
            }
            None if character == '\r' => out.push(' '),
            None => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Code, Error, Header, Stamp, render};
    use crate::parse::{ErrorCode, Location};

    const MESSAGE: &str =
        "MSH|^~\\&|LAB|NORTH|EHR|SOUTH|20260925120000+0200||ORU^R01^ORU_R01|MSG-1|P|2.5.1\rPID|1";

    fn stamp() -> Stamp<'static> {
        Stamp {
            control_id: "ACK-1",
            timestamp: "20260925120001+0200",
        }
    }

    #[test]
    fn an_accept_swaps_the_applications_and_echoes_msh_10() {
        let header = Header::from_text(MESSAGE).expect("a header");
        assert_eq!(
            render(&header, Code::Accept, &[], stamp()),
            "MSH|^~\\&|EHR|SOUTH|LAB|NORTH|20260925120001+0200||ACK^R01^ACK|ACK-1|P|2.5.1\rMSA|AA|MSG-1\r"
        );
    }

    #[test]
    fn an_error_carries_one_err_segment_per_refusal() {
        let header = Header::from_text(MESSAGE).expect("a header");
        let error = Error {
            location: Some(Location::segment("PID", 1).with_field(3)),
            code: ErrorCode::RequiredFieldMissing,
            text: String::from("PID.3 is required"),
        };
        let ack = render(&header, Code::Error, &[error], stamp());
        assert!(
            ack.ends_with("MSA|AE|MSG-1\rERR||PID^1^3|101^Required field missing^HL70357|E||||PID.3 is required\r"),
            "{ack}"
        );
    }

    #[test]
    fn the_type_code_joins_the_message_code_and_the_event() {
        let header = Header::from_text(MESSAGE).expect("a header");
        assert_eq!(header.type_code().as_deref(), Some("ORU^R01"));
        let bare = Header::from_text("MSH|^~\\&|LAB||||||ACK|MSG-2|P|2.5.1").expect("a header");
        assert_eq!(bare.type_code().as_deref(), Some("ACK"));
    }

    #[test]
    fn a_header_with_a_byte_outside_ascii_is_not_read() {
        assert!(Header::from_bytes(b"MSH|^~\\&|L\xC4B").is_none());
        assert!(Header::from_bytes(b"MSH|^~\\&|LAB").is_some());
    }
}
