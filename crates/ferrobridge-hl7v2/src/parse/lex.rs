// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The positional split of the decoded text: segments, fields, repetitions,
//! components and subcomponents, with the escape sequences decoded.

use crate::decode::Charset;
use crate::parse::{
    Component, Delimiters, ErrorCode, Field, Location, Message, Refusal, Repetition, Segment,
};

/// A refusal to read the text as an HL7 v2 message at all, which the
/// acknowledgment reports as `AR`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LexError {
    /// The first segment is not MSH.
    #[error("the message does not open with an MSH segment")]
    NoHeader,
    /// MSH-1 or MSH-2 does not declare the five delimiters.
    #[error("MSH-1 and MSH-2 do not declare distinct delimiters")]
    Delimiters,
    /// A segment id is not three upper-case letters or digits.
    #[error("segment {ordinal} has no valid segment id")]
    SegmentId {
        /// The segment's ordinal in the message, from 1.
        ordinal: usize,
    },
    /// A segment is empty.
    #[error("segment {ordinal} is empty")]
    EmptySegment {
        /// The segment's ordinal in the message, from 1.
        ordinal: usize,
    },
}

/// A lexed message with the refusals its values raised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed {
    /// The message.
    pub message: Message,
    /// A refusal per escape sequence this crate does not decode.
    pub refusals: Vec<Refusal>,
}

/// Splits `text` into segments, fields, repetitions, components and
/// subcomponents, decoding the escape sequences of each value.
///
/// # Errors
///
/// Returns [`LexError`] when the text opens with no MSH segment, when MSH-1
/// and MSH-2 declare no distinct delimiters, and when a segment is empty or
/// has no valid id.
pub fn lex(text: &str, charset: Charset) -> Result<Lexed, LexError> {
    let mut lines: Vec<&str> = text.split('\r').collect();
    if lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    let header = lines.first().ok_or(LexError::NoHeader)?;
    let delimiters = delimiters(header)?;
    let mut refusals = Vec::new();
    let mut segments = Vec::with_capacity(lines.len());
    let mut sequences: Vec<(String, usize)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let ordinal = index.saturating_add(1);
        if line.is_empty() {
            return Err(LexError::EmptySegment { ordinal });
        }
        let id = segment_id(line).ok_or(LexError::SegmentId { ordinal })?;
        if index == 0 && id != "MSH" {
            return Err(LexError::NoHeader);
        }
        let sequence = next_sequence(&mut sequences, id);
        let location = Location::segment(id, sequence);
        let rest = line.get(3..).unwrap_or_default();
        let mut lexer = Lexer {
            delimiters,
            charset,
            refusals: &mut refusals,
        };
        let fields = if id == "MSH" && index == 0 {
            lexer.header_fields(rest, &location)
        } else {
            let Some(body) = rest.strip_prefix(delimiters.field) else {
                if rest.is_empty() {
                    segments.push(Segment {
                        id: String::from(id),
                        fields: Vec::new(),
                    });
                    continue;
                }
                return Err(LexError::SegmentId { ordinal });
            };
            lexer.fields(body, &location, 1)
        };
        segments.push(Segment {
            id: String::from(id),
            fields,
        });
    }
    Ok(Lexed {
        message: Message {
            delimiters,
            charset,
            segments,
        },
        refusals,
    })
}

/// The sequence of the next segment with `id`, from 1.
fn next_sequence(sequences: &mut Vec<(String, usize)>, id: &str) -> usize {
    if let Some((_, count)) = sequences.iter_mut().find(|(seen, _)| seen == id) {
        *count = count.saturating_add(1);
        return *count;
    }
    sequences.push((String::from(id), 1));
    1
}

/// The segment id of a line: three upper-case letters or digits.
fn segment_id(line: &str) -> Option<&str> {
    let id = line.get(..3)?;
    id.bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        .then_some(id)
}

/// Reads MSH-1 and MSH-2 (chapter 2 §2.5.4: four encoding characters, a
/// fifth truncation character from v2.7 on).
fn delimiters(header: &str) -> Result<Delimiters, LexError> {
    if header.get(..3) != Some("MSH") {
        return Err(LexError::NoHeader);
    }
    let mut characters = header.get(3..).unwrap_or_default().chars();
    let field = characters.next().ok_or(LexError::Delimiters)?;
    let encoding: Vec<char> = characters
        .take_while(|character| *character != field)
        .collect();
    let [component, repetition, escape, subcomponent, rest @ ..] = encoding.as_slice() else {
        return Err(LexError::Delimiters);
    };
    let truncation = match rest {
        [] => None,
        [truncation] => Some(*truncation),
        _ => return Err(LexError::Delimiters),
    };
    let mut all = vec![field, *component, *repetition, *escape, *subcomponent];
    all.extend(truncation);
    let distinct = all.iter().enumerate().all(|(index, character)| {
        !all.iter()
            .skip(index.saturating_add(1))
            .any(|other| other == character)
    });
    if !distinct
        || all
            .iter()
            .any(|character| character.is_alphanumeric() || character.is_whitespace())
    {
        return Err(LexError::Delimiters);
    }
    Ok(Delimiters {
        field,
        component: *component,
        repetition: *repetition,
        escape: *escape,
        subcomponent: *subcomponent,
        truncation,
    })
}

/// The state of one segment's split.
struct Lexer<'a> {
    delimiters: Delimiters,
    charset: Charset,
    refusals: &'a mut Vec<Refusal>,
}

impl Lexer<'_> {
    /// Splits the MSH fields: MSH-1 and MSH-2 are one value each and never
    /// escaped.
    fn header_fields(&mut self, rest: &str, location: &Location) -> Vec<Field> {
        let separator = self.delimiters.field;
        let after_separator = rest.get(separator.len_utf8()..).unwrap_or_default();
        let (encoding, remainder) = match after_separator.split_once(separator) {
            Some((encoding, remainder)) => (encoding, Some(remainder)),
            None => (after_separator, None),
        };
        let mut fields = vec![literal(&separator.to_string()), literal(encoding)];
        if let Some(remainder) = remainder {
            fields.extend(self.fields(remainder, location, 3));
        }
        fields
    }

    /// Splits `body` into fields, the first at position `first`.
    fn fields(&mut self, body: &str, location: &Location, first: usize) -> Vec<Field> {
        let delimiters = self.delimiters;
        body.split(delimiters.field)
            .enumerate()
            .map(|(offset, text)| {
                let position = first.saturating_add(offset);
                let at = location.clone().with_field(position);
                Field {
                    repetitions: text
                        .split(delimiters.repetition)
                        .enumerate()
                        .map(|(repetition, text)| {
                            let mut at = at.clone();
                            at.repetition = Some(repetition.saturating_add(1));
                            self.repetition(text, &at)
                        })
                        .collect(),
                }
            })
            .collect()
    }

    /// Splits one repetition into components and subcomponents.
    fn repetition(&mut self, text: &str, at: &Location) -> Repetition {
        let delimiters = self.delimiters;
        Repetition {
            components: text
                .split(delimiters.component)
                .enumerate()
                .map(|(component, text)| Component {
                    subcomponents: text
                        .split(delimiters.subcomponent)
                        .enumerate()
                        .map(|(subcomponent, text)| {
                            let mut at = at.clone();
                            at.component = Some(component.saturating_add(1));
                            at.subcomponent = Some(subcomponent.saturating_add(1));
                            self.unescape(text, &at)
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Decodes the escape sequences of one value (chapter 2 §2.7).
    ///
    /// `\F\`, `\S\`, `\T\`, `\R\` and `\E\` become the delimiter they name and
    /// `\Xhh..\` the bytes it spells in the message's character set. The
    /// highlighting and formatting sequences (`\H\`, `\N\`, `\.br\` and the
    /// other `\.` commands) are text formatting of the `FT` type and stay in
    /// the value as written. `\Cxxyy\`, `\Mxxyyzz\` and the locally defined
    /// `\Z..\` are refused.
    fn unescape(&mut self, text: &str, at: &Location) -> String {
        let escape = self.delimiters.escape;
        if !text.contains(escape) {
            return String::from(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(escape) {
            out.push_str(rest.get(..start).unwrap_or_default());
            let after = rest
                .get(start.saturating_add(escape.len_utf8())..)
                .unwrap_or_default();
            let Some(end) = after.find(escape) else {
                self.refuse(at, "an escape sequence is not closed");
                out.push_str(rest.get(start..).unwrap_or_default());
                return out;
            };
            let sequence = after.get(..end).unwrap_or_default();
            match self.sequence(sequence) {
                Ok(Some(decoded)) => out.push_str(&decoded),
                Ok(None) => {
                    out.push(escape);
                    out.push_str(sequence);
                    out.push(escape);
                }
                Err(detail) => {
                    self.refuse(at, detail);
                    out.push(escape);
                    out.push_str(sequence);
                    out.push(escape);
                }
            }
            rest = after
                .get(end.saturating_add(escape.len_utf8())..)
                .unwrap_or_default();
        }
        out.push_str(rest);
        out
    }

    /// Decodes the body of one escape sequence: `Some` for a decoded value,
    /// `None` for a formatting sequence that stays as written.
    fn sequence(&self, sequence: &str) -> Result<Option<String>, &'static str> {
        let delimiters = self.delimiters;
        let one = |character: char| Ok(Some(character.to_string()));
        match sequence {
            "F" => one(delimiters.field),
            "S" => one(delimiters.component),
            "T" => one(delimiters.subcomponent),
            "R" => one(delimiters.repetition),
            "E" => one(delimiters.escape),
            "H" | "N" => Ok(None),
            _ if sequence.starts_with('.') => Ok(None),
            _ => match sequence.strip_prefix('X') {
                Some(hex) => self.hex(hex).map(Some),
                None => Err("an escape sequence this crate does not decode"),
            },
        }
    }

    /// Decodes `\Xhh..\` in the message's character set.
    fn hex(&self, hex: &str) -> Result<String, &'static str> {
        if hex.is_empty()
            || !hex.len().is_multiple_of(2)
            || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("a hexadecimal escape sequence is not an even run of hex digits");
        }
        let bytes: Option<Vec<u8>> = (0..hex.len())
            .step_by(2)
            .map(|offset| {
                hex.get(offset..offset.saturating_add(2))
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            })
            .collect();
        let bytes =
            bytes.ok_or("a hexadecimal escape sequence is not an even run of hex digits")?;
        match crate::decode::decode_bytes(&bytes, self.charset) {
            Ok(text) => Ok(text.into_owned()),
            Err(crate::decode::DecodeError::Undeclared { .. }) => {
                Err("a hexadecimal escape sequence spells bytes outside the character set")
            }
            Err(_) => Err("a hexadecimal escape sequence could not be decoded"),
        }
    }

    /// Records a refusal of the value at `at`.
    fn refuse(&mut self, at: &Location, detail: &str) {
        self.refusals.push(Refusal {
            location: at.clone(),
            code: ErrorCode::DataType,
            detail: String::from(detail),
        });
    }
}

/// A field holding one value as written.
fn literal(text: &str) -> Field {
    Field {
        repetitions: vec![Repetition {
            components: vec![Component {
                subcomponents: vec![String::from(text)],
            }],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::{LexError, delimiters};
    use crate::parse::Delimiters;

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn the_standard_encoding_characters_are_read_from_msh_2() -> Result<(), LexError> {
        let found = delimiters("MSH|^~\\&|APP")?;
        assert_eq!(
            found,
            Delimiters {
                field: '|',
                component: '^',
                repetition: '~',
                escape: '\\',
                subcomponent: '&',
                truncation: None,
            }
        );
        assert_eq!(found.encoding_characters(), "^~\\&");
        assert_eq!(delimiters("MSH|^~\\&#|")?.truncation, Some('#'));
        Ok(())
    }

    #[test]
    fn repeated_delimiters_are_refused() {
        assert_eq!(delimiters("MSH|^^\\&|"), Err(LexError::Delimiters));
        assert_eq!(delimiters("MSH|^~\\|"), Err(LexError::Delimiters));
    }
}
