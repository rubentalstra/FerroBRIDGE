// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Decoding a message from the character set its MSH-18 names.
//!
//! MSH-18 names the character set of the whole message from HL7 table 0211
//! (`CodeSystem/v2-0211` in the vendored `hl7.terminology` package). The MSH
//! segment up to MSH-18 is read as bytes before the message is decoded, since
//! the field separator, the encoding characters and the table 0211 codes are
//! all 7-bit. An empty MSH-18 falls back to the configured default, because
//! the MLLP ends of a connection "mutually agree" on the encoding (MLLP
//! Release 1, §Block format).
//!
//! A byte sequence the declared set does not define is a typed refusal, never
//! a replacement character. Table 0211 names "the printable characters" of
//! each ISO 8859 part, so the C1 control range `0x80` to `0x9F` is refused for
//! every part.

use std::borrow::Cow;

use encoding_rs::Encoding;

/// A character set of HL7 table 0211 this crate decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Charset {
    /// `ASCII`, the printable 7-bit ASCII character set.
    Ascii,
    /// `8859/1` to `8859/9` and `8859/15`, one ISO 8859 part.
    Iso8859(u8),
    /// `UNICODE UTF-8`.
    Utf8,
}

impl Charset {
    /// Returns the charset a table 0211 code names, `None` for a code this
    /// crate does not decode.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "ASCII" => Some(Self::Ascii),
            "UNICODE UTF-8" => Some(Self::Utf8),
            "8859/1" => Some(Self::Iso8859(1)),
            "8859/2" => Some(Self::Iso8859(2)),
            "8859/3" => Some(Self::Iso8859(3)),
            "8859/4" => Some(Self::Iso8859(4)),
            "8859/5" => Some(Self::Iso8859(5)),
            "8859/6" => Some(Self::Iso8859(6)),
            "8859/7" => Some(Self::Iso8859(7)),
            "8859/8" => Some(Self::Iso8859(8)),
            "8859/9" => Some(Self::Iso8859(9)),
            "8859/15" => Some(Self::Iso8859(15)),
            _ => None,
        }
    }

    /// Returns the table 0211 code of this charset.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::Utf8 => "UNICODE UTF-8",
            Self::Iso8859(1) => "8859/1",
            Self::Iso8859(2) => "8859/2",
            Self::Iso8859(3) => "8859/3",
            Self::Iso8859(4) => "8859/4",
            Self::Iso8859(5) => "8859/5",
            Self::Iso8859(6) => "8859/6",
            Self::Iso8859(7) => "8859/7",
            Self::Iso8859(8) => "8859/8",
            Self::Iso8859(9) => "8859/9",
            Self::Iso8859(_) => "8859/15",
        }
    }

    /// The `encoding_rs` encoding for an ISO 8859 part, `None` for the parts
    /// decoded another way.
    ///
    /// `encoding_rs` implements the WHATWG Encoding Standard, which decodes the
    /// labels `iso-8859-1` and `iso-8859-9` as windows-1252 and windows-1254
    /// (<https://docs.rs/encoding_rs/0.8.42/encoding_rs/#iso-8859-1>). With the
    /// C1 range refused first, windows-1254 agrees with ISO 8859-9 on every
    /// byte left, and ISO 8859-1 goes through `mem::decode_latin1`.
    fn iso_encoding(part: u8) -> Option<&'static Encoding> {
        match part {
            2 => Some(encoding_rs::ISO_8859_2),
            3 => Some(encoding_rs::ISO_8859_3),
            4 => Some(encoding_rs::ISO_8859_4),
            5 => Some(encoding_rs::ISO_8859_5),
            6 => Some(encoding_rs::ISO_8859_6),
            7 => Some(encoding_rs::ISO_8859_7),
            8 => Some(encoding_rs::ISO_8859_8),
            9 => Some(encoding_rs::WINDOWS_1254),
            15 => Some(encoding_rs::ISO_8859_15),
            _ => None,
        }
    }
}

/// A refusal to decode a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The message does not open with an `MSH` segment.
    #[error("the message does not open with an MSH segment")]
    NoHeader,
    /// MSH-18 names a character set this crate does not decode.
    #[error("MSH-18 names the character set {code:?}, which is not decoded")]
    UnsupportedCharset {
        /// The code as the message wrote it.
        code: String,
    },
    /// MSH-18 repeats, which names alternate character sets (MSH-20).
    #[error("MSH-18 repeats; alternate character set handling is not supported")]
    AlternateCharsets,
    /// A byte sequence the declared character set does not define.
    #[error("the byte at offset {offset} is outside the character set {charset}")]
    Undeclared {
        /// The character set the message declared or the connection agreed.
        charset: &'static str,
        /// The offset of the first offending byte.
        offset: usize,
    },
}

/// A message decoded to text, with the character set it was read in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The message text.
    pub text: String,
    /// The character set it was read in.
    pub charset: Charset,
    /// Whether MSH-18 named the set, as opposed to the connection default.
    pub declared: bool,
}

/// Decodes `message` from the character set its MSH-18 names, or from
/// `default` when MSH-18 is empty.
///
/// # Errors
///
/// Returns [`DecodeError`] when the message opens with no MSH segment, when
/// MSH-18 names a set this crate does not decode or repeats, and when a byte
/// sequence falls outside the set.
pub fn decode(message: &[u8], default: Charset) -> Result<Decoded, DecodeError> {
    let (charset, declared) = match declared_charset(message)? {
        Some(code) => (
            Charset::from_code(&code).ok_or(DecodeError::UnsupportedCharset { code })?,
            true,
        ),
        None => (default, false),
    };
    let text = decode_as(message, charset)?.into_owned();
    Ok(Decoded {
        text,
        charset,
        declared,
    })
}

/// Encodes `text` in `charset`, for an acknowledgment answered in the set the
/// message arrived in.
///
/// # Errors
///
/// Returns [`DecodeError::Undeclared`] when a character has no byte in the
/// set, at the character's offset in `text`.
pub fn encode(text: &str, charset: Charset) -> Result<Vec<u8>, DecodeError> {
    let undeclared = |offset: usize| DecodeError::Undeclared {
        charset: charset.code(),
        offset,
    };
    match charset {
        Charset::Utf8 => Ok(text.as_bytes().to_vec()),
        Charset::Ascii => match text.bytes().position(|byte| !byte.is_ascii()) {
            Some(offset) => Err(undeclared(offset)),
            None => Ok(text.as_bytes().to_vec()),
        },
        Charset::Iso8859(1) => {
            let mut out = Vec::with_capacity(text.len());
            for (offset, character) in text.char_indices() {
                let Ok(byte) = u8::try_from(u32::from(character)) else {
                    return Err(undeclared(offset));
                };
                out.push(byte);
            }
            Ok(out)
        }
        Charset::Iso8859(part) => {
            let encoding = Charset::iso_encoding(part).ok_or_else(|| undeclared(0))?;
            let (bytes, _, unmappable) = encoding.encode(text);
            if unmappable {
                return Err(undeclared(0));
            }
            Ok(bytes.into_owned())
        }
    }
}

/// Reads MSH-18 from the raw bytes: `None` when it is empty or absent.
fn declared_charset(message: &[u8]) -> Result<Option<String>, DecodeError> {
    let header_end = message
        .iter()
        .position(|byte| *byte == b'\r')
        .unwrap_or(message.len());
    let header = message.get(..header_end).unwrap_or_default();
    if header.get(..3) != Some(b"MSH".as_slice()) {
        return Err(DecodeError::NoHeader);
    }
    let separator = *header.get(3).ok_or(DecodeError::NoHeader)?;
    let repetition = *header.get(5).ok_or(DecodeError::NoHeader)?;
    // NOTE: HL7 v2.5.1 chapter 2 §2.14.9.1: MSH-1 is the separator itself, so
    // the n-th piece after splitting on it is MSH-(n+1).
    let Some(field) = header.split(|byte| *byte == separator).nth(17) else {
        return Ok(None);
    };
    if field.is_empty() {
        return Ok(None);
    }
    if field.contains(&repetition) {
        return Err(DecodeError::AlternateCharsets);
    }
    match core::str::from_utf8(field) {
        Ok(code) if field.is_ascii() => Ok(Some(String::from(code))),
        _ => Err(DecodeError::UnsupportedCharset {
            code: String::from_utf8_lossy(field).into_owned(),
        }),
    }
}

/// Decodes `bytes` in `charset`, refusing any byte outside it: the bytes a
/// `\Xhh..\` escape sequence spells.
pub(crate) fn decode_bytes(bytes: &[u8], charset: Charset) -> Result<Cow<'_, str>, DecodeError> {
    decode_as(bytes, charset)
}

/// Decodes `message` in `charset`, refusing any byte outside it.
fn decode_as(message: &[u8], charset: Charset) -> Result<Cow<'_, str>, DecodeError> {
    let undeclared = |offset: usize| DecodeError::Undeclared {
        charset: charset.code(),
        offset,
    };
    match charset {
        Charset::Utf8 => core::str::from_utf8(message)
            .map(Cow::Borrowed)
            .map_err(|error| undeclared(error.valid_up_to())),
        Charset::Ascii => match message.iter().position(|byte| !byte.is_ascii()) {
            Some(offset) => Err(undeclared(offset)),
            None => core::str::from_utf8(message)
                .map(Cow::Borrowed)
                .map_err(|error| undeclared(error.valid_up_to())),
        },
        Charset::Iso8859(part) => {
            if let Some(offset) = message.iter().position(|byte| (0x80..=0x9F).contains(byte)) {
                return Err(undeclared(offset));
            }
            if part == 1 {
                return Ok(encoding_rs::mem::decode_latin1(message));
            }
            let encoding = Charset::iso_encoding(part).ok_or_else(|| undeclared(0))?;
            encoding
                .decode_without_bom_handling_and_without_replacement(message)
                .ok_or_else(|| undeclared(first_undefined(encoding, message)))
        }
    }
}

/// The offset of the first byte `encoding` does not define.
fn first_undefined(encoding: &'static Encoding, message: &[u8]) -> usize {
    (0..message.len())
        .find(|offset| {
            message.get(*offset..=*offset).is_some_and(|byte| {
                encoding
                    .decode_without_bom_handling_and_without_replacement(byte)
                    .is_none()
            })
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Charset, DecodeError, decode, encode};

    #[test]
    fn every_table_0211_code_this_crate_decodes_round_trips_its_code() {
        for code in [
            "ASCII",
            "UNICODE UTF-8",
            "8859/1",
            "8859/2",
            "8859/5",
            "8859/9",
            "8859/15",
        ] {
            assert_eq!(Charset::from_code(code).map(Charset::code), Some(code));
        }
        assert_eq!(Charset::from_code("UNICODE UTF-16"), None);
    }

    /// A header whose MSH-18 is `charset`, followed by `rest`.
    fn message(charset: &str, rest: &[u8]) -> Vec<u8> {
        let mut bytes = format!("MSH|^~\\&{}{charset}\r", "|".repeat(16)).into_bytes();
        bytes.extend_from_slice(rest);
        bytes
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn iso_8859_9_decodes_its_turkish_letters() -> Result<(), DecodeError> {
        let decoded = decode(&message("8859/9", b"PID|\xDEEN\xDDZ"), Charset::Ascii)?;
        assert!(decoded.declared, "MSH-18 named the set");
        assert!(
            decoded.text.ends_with("\u{15E}EN\u{130}Z"),
            "{}",
            decoded.text
        );
        Ok(())
    }

    #[test]
    fn an_undefined_iso_8859_3_byte_is_refused_at_its_offset() {
        let bytes = message("8859/3", b"PID|\xA5");
        let offset = bytes.len().saturating_sub(1);
        assert_eq!(
            decode(&bytes, Charset::Ascii),
            Err(DecodeError::Undeclared {
                charset: "8859/3",
                offset,
            })
        );
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn latin1_encodes_back_to_its_bytes() -> Result<(), DecodeError> {
        assert_eq!(
            encode("J\u{F6}rg", Charset::Iso8859(1))?,
            b"J\xF6rg".to_vec()
        );
        assert!(encode("\u{20AC}", Charset::Iso8859(1)).is_err());
        Ok(())
    }
}
