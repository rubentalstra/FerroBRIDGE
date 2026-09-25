// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Parsing a message by position and grouping it by its message structure.
//!
//! [`lex`] splits the decoded text by the HL7 encoding rules (HL7 v2.5.1
//! chapter 2 §2.5 and §2.7): segments on the carriage return, fields on the
//! MSH-1 separator, and repetitions, components and subcomponents on the
//! MSH-2 encoding characters, with the escape sequences of §2.7 decoded in
//! each value. [`group`] then places each segment in the segment-group tree
//! of the message structure the generated `hl7v2-types` tables define.
//!
//! A segment the definitions do not know (a Z-segment, a local one), a known
//! segment the structure has no place for, and a valued field beyond a
//! segment's table are counted outcomes ([`Unplaced`]). A missing required
//! segment or field and an escape sequence this crate does not decode are
//! refusals ([`Refusal`]), which the acknowledgment reports as `AE` with one
//! `ERR` segment each.

use core::fmt;

use hl7v2_types::model::{Group, GroupKind, Max, Node, Optionality, SegmentRef, Structure};

use crate::decode::Charset;

/// The delimiters MSH-1 and MSH-2 declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delimiters {
    /// MSH-1, the field separator.
    pub field: char,
    /// The component separator, the first character of MSH-2.
    pub component: char,
    /// The repetition separator, the second character of MSH-2.
    pub repetition: char,
    /// The escape character, the third character of MSH-2.
    pub escape: char,
    /// The subcomponent separator, the fourth character of MSH-2.
    pub subcomponent: char,
    /// The truncation character, the fifth character of MSH-2 from v2.7 on.
    pub truncation: Option<char>,
}

impl Delimiters {
    /// Renders MSH-2 as the delimiters declare it.
    #[must_use]
    pub fn encoding_characters(&self) -> String {
        let mut text = String::new();
        text.push(self.component);
        text.push(self.repetition);
        text.push(self.escape);
        text.push(self.subcomponent);
        if let Some(truncation) = self.truncation {
            text.push(truncation);
        }
        text
    }

    /// Escapes `text` so it travels as one value (chapter 2 §2.7.1).
    #[must_use]
    pub fn escape(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for character in text.chars() {
            let code = if character == self.field {
                Some('F')
            } else if character == self.component {
                Some('S')
            } else if character == self.subcomponent {
                Some('T')
            } else if character == self.repetition {
                Some('R')
            } else if character == self.escape {
                Some('E')
            } else {
                None
            };
            match code {
                Some(code) => {
                    out.push(self.escape);
                    out.push(code);
                    out.push(self.escape);
                }
                None => out.push(character),
            }
        }
        out
    }
}

/// One segment of a message, its fields by position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    id: String,
    fields: Vec<Field>,
}

impl Segment {
    /// Returns the segment id, for example `PID`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the field at `position`, from 1.
    ///
    /// For MSH, field 1 is the field separator and field 2 the encoding
    /// characters, each one value (chapter 2 §2.14.9.1 and §2.14.9.2).
    #[must_use]
    pub fn field(&self, position: usize) -> Option<&Field> {
        self.fields.get(position.checked_sub(1)?)
    }

    /// Returns the fields in position order.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Returns the first value of the field at `position`, `None` when it is
    /// not valued.
    #[must_use]
    pub fn text(&self, position: usize) -> Option<&str> {
        self.field(position).and_then(Field::text)
    }
}

/// One field: its repetitions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Field {
    repetitions: Vec<Repetition>,
}

impl Field {
    /// Returns the repetitions, in message order.
    #[must_use]
    pub fn repetitions(&self) -> &[Repetition] {
        &self.repetitions
    }

    /// Returns whether any repetition holds a value.
    #[must_use]
    pub fn is_valued(&self) -> bool {
        self.repetitions.iter().any(Repetition::is_valued)
    }

    /// Returns the first subcomponent of the first repetition, `None` when it
    /// is not valued.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.repetitions.first().and_then(Repetition::text)
    }
}

/// One repetition of a field: its components.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repetition {
    components: Vec<Component>,
}

impl Repetition {
    /// Returns the components, from component 1.
    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    /// Returns the component at `position`, from 1.
    #[must_use]
    pub fn component(&self, position: usize) -> Option<&Component> {
        self.components.get(position.checked_sub(1)?)
    }

    /// Returns whether any subcomponent holds a value.
    #[must_use]
    pub fn is_valued(&self) -> bool {
        self.components.iter().any(Component::is_valued)
    }

    /// Returns the first subcomponent of the first component, `None` when it
    /// is not valued.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.components.first().and_then(Component::text)
    }
}

/// One component: its subcomponents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Component {
    subcomponents: Vec<String>,
}

impl Component {
    /// Returns the subcomponents, from subcomponent 1.
    #[must_use]
    pub fn subcomponents(&self) -> &[String] {
        &self.subcomponents
    }

    /// Returns the subcomponent at `position`, from 1, `None` when it is not
    /// valued.
    #[must_use]
    pub fn subcomponent(&self, position: usize) -> Option<&str> {
        self.subcomponents
            .get(position.checked_sub(1)?)
            .map(String::as_str)
            .filter(|value| valued(value))
    }

    /// Returns whether any subcomponent holds a value.
    #[must_use]
    pub fn is_valued(&self) -> bool {
        self.subcomponents.iter().any(|value| valued(value))
    }

    /// Returns the first subcomponent, `None` when it is not valued.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.subcomponent(1)
    }
}

/// Whether a value is present and not the explicit null `""`.
///
/// Chapter 2 §2.5.3 gives `""` the meaning "the value is null", so it carries
/// no value to map.
#[must_use]
pub fn valued(value: &str) -> bool {
    !value.is_empty() && value != "\"\""
}

/// A lexed message: its delimiters and segments, with the character set it
/// was read in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    delimiters: Delimiters,
    charset: Charset,
    segments: Vec<Segment>,
}

impl Message {
    /// Returns the delimiters MSH-1 and MSH-2 declare.
    #[must_use]
    pub const fn delimiters(&self) -> &Delimiters {
        &self.delimiters
    }

    /// Returns the character set the message was read in.
    #[must_use]
    pub const fn charset(&self) -> Charset {
        self.charset
    }

    /// Returns the segments in message order, MSH first.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Returns the MSH segment.
    #[must_use]
    pub fn header(&self) -> Option<&Segment> {
        self.segments.first().filter(|segment| segment.id == "MSH")
    }

    /// Returns the component at `component` of MSH-9, from 1: the message
    /// code (1), the trigger event (2) or the message structure (3).
    #[must_use]
    pub fn message_type(&self, component: usize) -> Option<&str> {
        self.header()?
            .field(9)?
            .repetitions()
            .first()?
            .component(component)?
            .text()
    }

    /// Returns MSH-10, the message control id.
    #[must_use]
    pub fn control_id(&self) -> Option<&str> {
        self.header()?.text(10)
    }

    /// Returns MSH-12.1, the version id.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.header()?.text(12)
    }
}

/// The error codes of HL7 table 0357 a refusal carries
/// (`CodeSystem/v2-0357` in the vendored `hl7.terminology` package).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorCode {
    /// `100`, segment sequence error.
    SegmentSequence,
    /// `101`, required field missing.
    RequiredFieldMissing,
    /// `102`, data type error.
    DataType,
    /// `200`, unsupported message type.
    UnsupportedMessageType,
    /// `203`, unsupported version id.
    UnsupportedVersion,
    /// `207`, application error.
    Application,
}

impl ErrorCode {
    /// Returns the table 0357 code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::SegmentSequence => "100",
            Self::RequiredFieldMissing => "101",
            Self::DataType => "102",
            Self::UnsupportedMessageType => "200",
            Self::UnsupportedVersion => "203",
            Self::Application => "207",
        }
    }

    /// Returns the table 0357 display.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::SegmentSequence => "Segment sequence error",
            Self::RequiredFieldMissing => "Required field missing",
            Self::DataType => "Data type error",
            Self::UnsupportedMessageType => "Unsupported message type",
            Self::UnsupportedVersion => "Unsupported version id",
            Self::Application => "Application error",
        }
    }
}

/// Where in a message a refusal or an outcome is, the components of the ERL
/// data type ERR-2 carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location {
    /// The segment id.
    pub segment: String,
    /// The segment's sequence among the segments with that id, from 1.
    pub sequence: usize,
    /// The field position, from 1.
    pub field: Option<usize>,
    /// The field repetition, from 1.
    pub repetition: Option<usize>,
    /// The component, from 1.
    pub component: Option<usize>,
    /// The subcomponent, from 1.
    pub subcomponent: Option<usize>,
}

impl Location {
    /// Creates the location of a whole segment.
    #[must_use]
    pub fn segment(id: &str, sequence: usize) -> Self {
        Self {
            segment: String::from(id),
            sequence,
            ..Self::default()
        }
    }

    /// Returns this location narrowed to `field`.
    #[must_use]
    pub fn with_field(mut self, field: usize) -> Self {
        self.field = Some(field);
        self
    }

    /// Renders the location as the ERL components with `separator` between
    /// them, the trailing unknown ones omitted.
    #[must_use]
    pub fn erl(&self, separator: char) -> String {
        let mut parts = vec![self.segment.clone(), self.sequence.to_string()];
        for part in [
            self.field,
            self.repetition,
            self.component,
            self.subcomponent,
        ] {
            match part {
                Some(value) => parts.push(value.to_string()),
                None => break,
            }
        }
        parts.join(&separator.to_string())
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.segment, self.sequence)?;
        if let Some(field) = self.field {
            write!(f, "-{field}")?;
        }
        if let Some(repetition) = self.repetition {
            write!(f, "({repetition})")?;
        }
        if let Some(component) = self.component {
            write!(f, ".{component}")?;
        }
        if let Some(subcomponent) = self.subcomponent {
            write!(f, ".{subcomponent}")?;
        }
        Ok(())
    }
}

/// A refusal the acknowledgment reports as `AE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Where.
    pub location: Location,
    /// The table 0357 code.
    pub code: ErrorCode,
    /// What was refused, without message content.
    pub detail: String,
}

/// A segment or field the parse does not carry, counted on the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unplaced {
    /// A segment the definitions do not know: a Z-segment or a local one.
    UnknownSegment {
        /// Where.
        location: Location,
    },
    /// A segment the definitions know but the message structure has no place
    /// for at this point of the message.
    OutOfStructure {
        /// Where.
        location: Location,
    },
    /// A valued field beyond the segment's field table.
    ExtraField {
        /// Where.
        location: Location,
    },
    /// The message declares an earlier version than the definitions, and is
    /// parsed against them.
    EarlierVersion {
        /// MSH-12.1 as the message wrote it.
        declared: String,
    },
    /// MSH-9.3 names a structure other than the one the message is grouped
    /// by: one the definitions lack, where the message definition of MSH-9.1
    /// and MSH-9.2 names the structure ([`structure_for`]).
    OtherStructure {
        /// MSH-9.3 as the message wrote it.
        declared: String,
        /// The id of the structure the message is grouped by.
        structure: &'static str,
    },
}

impl Unplaced {
    /// Returns a short name of the outcome's kind, for a count.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnknownSegment { .. } => "unknown-segment",
            Self::OutOfStructure { .. } => "out-of-structure",
            Self::ExtraField { .. } => "extra-field",
            Self::EarlierVersion { .. } => "earlier-version",
            Self::OtherStructure { .. } => "other-structure",
        }
    }
}

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

/// Why a structure could not be selected for a message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StructureError {
    /// MSH-9.3 is not valued and the message index names no structure for
    /// MSH-9.1 and MSH-9.2.
    #[error(
        "MSH-9.3 names no message structure and none is defined for the message type and trigger event"
    )]
    Unnamed,
    /// MSH-9.3 names no structure of the definitions.
    #[error("MSH-9.3 names the unknown message structure {name:?}")]
    Unknown {
        /// MSH-9.3 as the message wrote it.
        name: String,
    },
    /// MSH-9.3 names a structure the definitions carry in several variants,
    /// and no message definition for MSH-9.1 and MSH-9.2 names one of them.
    #[error("the message structure {name:?} has the variants {candidates:?}")]
    Variant {
        /// MSH-9.3 as the message wrote it.
        name: String,
        /// The variant ids.
        candidates: Vec<&'static str>,
    },
}

/// Selects the message structure MSH-9.3 names.
///
/// A structure the definitions carry once (`ACK`, `ADT_A02`) is selected by
/// its id. One they carry in variants (`ORU_R01-A` to `ORU_R01-D`) is
/// selected by the message definition of MSH-9.1 and MSH-9.2
/// ([`hl7v2_types::message::find`]): `ORU^R01` names `ORU_R01-A` and
/// `ADT^A04` names `ADT_A01-B`. When MSH-9.3 names no structure of the
/// definitions (`ADT^A08^ADT_A08`), the message definition of MSH-9.1 and
/// MSH-9.2 selects it (`ADT_A01`), and [`group`] counts
/// [`Unplaced::OtherStructure`].
///
/// # Errors
///
/// Returns [`StructureError`] when MSH-9.3 is empty or names no structure
/// and no message definition for MSH-9.1 and MSH-9.2 names one, or when
/// MSH-9.3 names one with variants of which that definition names none.
pub fn structure_for(message: &Message) -> Result<&'static Structure, StructureError> {
    let indexed = message
        .message_type(1)
        .zip(message.message_type(2))
        .and_then(|(code, event)| hl7v2_types::message::find(code, event))
        .and_then(|definition| definition.structure);
    let Some(name) = message.message_type(3) else {
        // NOTE: HL7 v2.5.1 chapter 2 §2.15.9.9 makes MSH-9.3 optional where table
        // 0354 fixes the structure, so the message index answers for a header without it.
        return indexed.ok_or(StructureError::Unnamed);
    };
    if let Some(structure) = hl7v2_types::structure::find(name) {
        return Ok(structure);
    }
    let prefix = format!("{name}-");
    let candidates: Vec<&'static str> = hl7v2_types::structure::STRUCTURES
        .iter()
        .map(|structure| structure.id)
        .filter(|id| id.starts_with(&prefix))
        .collect();
    if candidates.is_empty() {
        // NOTE: HL7 v2.5.1 chapter 2 §2.15.9.9: table 0354 fixes the structure from MSH-9.1
        // and MSH-9.2, so the index answers for an MSH-9.3 naming none, counted by `group`.
        return indexed.ok_or_else(|| StructureError::Unknown {
            name: String::from(name),
        });
    }
    let named = indexed.filter(|structure| candidates.contains(&structure.id));
    named.ok_or_else(|| StructureError::Variant {
        name: String::from(name),
        candidates,
    })
}

/// A segment placed in the structure.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The index of the segment in [`Message::segments`].
    pub index: usize,
    /// The node of the structure the segment fills.
    pub node: &'static SegmentRef,
    /// The segment's occurrence at that node, from 0.
    pub occurrence: usize,
}

/// One instance of a segment group.
#[derive(Debug, Clone)]
pub struct Instance {
    /// The group definition.
    pub group: &'static Group,
    /// The instance's occurrence at its node, from 0.
    pub occurrence: usize,
    /// The segments and group instances inside it, in message order.
    pub items: Vec<Item>,
}

/// One entry of a grouped message.
#[derive(Debug, Clone)]
pub enum Item {
    /// A placed segment.
    Segment(Placed),
    /// A group instance.
    Group(Instance),
}

/// A message grouped by its structure.
#[derive(Debug, Clone)]
pub struct Parsed {
    message: Message,
    structure: &'static Structure,
    items: Vec<Item>,
    unplaced: Vec<Unplaced>,
    refusals: Vec<Refusal>,
}

impl Parsed {
    /// Returns the message.
    #[must_use]
    pub const fn message(&self) -> &Message {
        &self.message
    }

    /// Returns the structure the message was grouped by.
    #[must_use]
    pub const fn structure(&self) -> &'static Structure {
        self.structure
    }

    /// Returns the structure's name without its variant suffix, for example
    /// `ORU_R01` for `ORU_R01-A`.
    #[must_use]
    pub fn structure_name(&self) -> &'static str {
        base_name(self.structure.id)
    }

    /// Returns the top-level segments and group instances.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Returns the segments and fields the parse does not carry.
    #[must_use]
    pub fn unplaced(&self) -> &[Unplaced] {
        &self.unplaced
    }

    /// Returns the refusals, lexical ones first.
    #[must_use]
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }

    /// Returns the location of the segment at `index` of the message.
    #[must_use]
    pub fn location(&self, index: usize) -> Location {
        let segments = self.message.segments();
        let Some(segment) = segments.get(index) else {
            return Location::default();
        };
        let sequence = segments
            .iter()
            .take(index.saturating_add(1))
            .filter(|other| other.id == segment.id)
            .count();
        Location::segment(&segment.id, sequence)
    }
}

/// The name of a structure id without its variant suffix.
fn base_name(id: &'static str) -> &'static str {
    match id.rsplit_once('-') {
        Some((base, variant)) if variant.len() == 1 => base,
        _ => id,
    }
}

/// Groups `lexed` by `structure`.
///
/// Each segment is placed at the next node of the structure that can hold it,
/// searching forward from the node the previous segment filled: a later
/// sibling, a new instance of a repeating group whose first segments include
/// it, or a sibling of an enclosing group. A segment with no such node is
/// counted as [`Unplaced::OutOfStructure`], and the cursor stays where it was.
/// A required segment or group left empty, and a required field left empty in
/// a placed segment, are refusals.
#[must_use]
pub fn group(lexed: Lexed, structure: &'static Structure) -> Parsed {
    let Lexed { message, refusals } = lexed;
    let mut parsed = Parsed {
        message,
        structure,
        items: Vec::new(),
        unplaced: Vec::new(),
        refusals,
    };
    if let Some(declared) = parsed.message.version()
        && declared != crate::DEFINITIONS_VERSION
    {
        parsed.unplaced.push(Unplaced::EarlierVersion {
            declared: String::from(declared),
        });
    }
    if let Some(declared) = parsed.message.message_type(3)
        && declared != base_name(structure.id)
    {
        parsed.unplaced.push(Unplaced::OtherStructure {
            declared: String::from(declared),
            structure: structure.id,
        });
    }
    let mut stack = vec![Frame::root(structure.nodes)];
    let mut missing = Vec::new();
    for index in 0..parsed.message.segments.len() {
        let location = parsed.location(index);
        let Some(segment) = parsed.message.segments.get(index) else {
            continue;
        };
        if hl7v2_types::segment::find(&segment.id).is_none() {
            parsed.unplaced.push(Unplaced::UnknownSegment { location });
            continue;
        }
        if !place(&mut stack, &segment.id, index, &mut missing) {
            parsed.unplaced.push(Unplaced::OutOfStructure { location });
        }
    }
    while stack.len() > 1 {
        close(&mut stack, &mut missing);
    }
    if let Some(root) = stack.pop() {
        root.check(&mut missing);
        parsed.items = root.items;
    }
    for name in missing {
        parsed.refusals.push(Refusal {
            location: Location::default(),
            code: ErrorCode::SegmentSequence,
            detail: format!("the required {name} is missing"),
        });
    }
    let placed = placed_segments(&parsed.items);
    for (index, node) in placed {
        check_fields(&mut parsed, index, node);
    }
    parsed
}

/// Every placed segment with the node it fills, in message order.
fn placed_segments(items: &[Item]) -> Vec<(usize, &'static SegmentRef)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Segment(placed) => out.push((placed.index, placed.node)),
            Item::Group(instance) => out.extend(placed_segments(&instance.items)),
        }
    }
    out.sort_by_key(|(index, _)| *index);
    out
}

/// Refuses a missing required field and counts a valued field beyond the
/// segment's table.
fn check_fields(parsed: &mut Parsed, index: usize, node: &'static SegmentRef) {
    let location = parsed.location(index);
    let Some(segment) = parsed.message.segments.get(index) else {
        return;
    };
    let definition = node.segment;
    for field in definition.fields {
        let position = usize::from(field.position);
        if field.optionality == Optionality::R
            && !segment.field(position).is_some_and(Field::is_valued)
        {
            parsed.refusals.push(Refusal {
                location: location.clone().with_field(position),
                code: ErrorCode::RequiredFieldMissing,
                detail: format!("{} is required", field.id),
            });
        }
    }
    for (offset, field) in segment
        .fields
        .iter()
        .enumerate()
        .skip(definition.fields.len())
    {
        if field.is_valued() {
            parsed.unplaced.push(Unplaced::ExtraField {
                location: location.clone().with_field(offset.saturating_add(1)),
            });
        }
    }
}

/// One open group instance while the segments are placed.
struct Frame {
    nodes: &'static [Node],
    group: Option<&'static Group>,
    occurrence: usize,
    at: usize,
    counts: Vec<usize>,
    items: Vec<Item>,
}

impl Frame {
    /// The frame of the structure's top level.
    fn root(nodes: &'static [Node]) -> Self {
        Self {
            nodes,
            group: None,
            occurrence: 0,
            at: 0,
            counts: vec![0; nodes.len()],
            items: Vec::new(),
        }
    }

    /// The frame of a new instance of `group`.
    fn instance(group: &'static Group, occurrence: usize) -> Self {
        Self {
            nodes: group.children,
            group: Some(group),
            occurrence,
            at: 0,
            counts: vec![0; group.children.len()],
            items: Vec::new(),
        }
    }

    /// Whether this frame is a choice group whose alternative is already
    /// taken, so only that alternative can take more segments.
    fn locked(&self) -> bool {
        self.group
            .is_some_and(|group| group.kind == GroupKind::Choice)
            && self.counts.iter().any(|count| *count > 0)
    }

    /// Records a required node left empty.
    fn check(&self, missing: &mut Vec<String>) {
        if self
            .group
            .is_some_and(|group| group.kind == GroupKind::Choice)
        {
            return;
        }
        for (node, count) in self.nodes.iter().zip(&self.counts) {
            if *count > 0 {
                continue;
            }
            match node {
                Node::Segment(reference) if reference.cardinality.min > 0 => {
                    missing.push(format!("segment {}", reference.id));
                }
                Node::Group(group) if group.cardinality.min > 0 => {
                    missing.push(format!("group {}", group.id));
                }
                Node::Segment(_) | Node::Group(_) | Node::Placeholder(_) => {}
            }
        }
    }
}

/// Whether one more occurrence fits under `max` after `count`.
fn fits(count: usize, max: Max) -> bool {
    match max {
        Max::Unbounded => true,
        Max::Bounded(bound) => usize::try_from(bound).map_or(true, |bound| count < bound),
    }
}

/// Places the segment `id` at `index`, returning whether a node took it.
fn place(stack: &mut Vec<Frame>, id: &str, index: usize, missing: &mut Vec<String>) -> bool {
    for depth in (0..stack.len()).rev() {
        let Some(frame) = stack.get(depth) else {
            continue;
        };
        let Some(found) = candidate(frame, id) else {
            continue;
        };
        while stack.len() > depth.saturating_add(1) {
            close(stack, missing);
        }
        return enter(stack, found, id, index);
    }
    false
}

/// The node of `frame` that can take `id`, searching from the current one.
fn candidate(frame: &Frame, id: &str) -> Option<usize> {
    let last = if frame.locked() {
        frame.at
    } else {
        frame.nodes.len().saturating_sub(1)
    };
    (frame.at..=last).find(|position| {
        let count = frame.counts.get(*position).copied().unwrap_or_default();
        match frame.nodes.get(*position) {
            Some(Node::Segment(reference)) => {
                reference.segment.id == id && fits(count, reference.cardinality.max)
            }
            Some(Node::Group(group)) => {
                fits(count, group.cardinality.max) && starts(&group_first(group), id)
            }
            Some(Node::Placeholder(_)) | None => false,
        }
    })
}

/// Takes node `position` of the top frame for the segment, entering groups
/// down to the segment node.
fn enter(stack: &mut Vec<Frame>, position: usize, id: &str, index: usize) -> bool {
    let Some(frame) = stack.last_mut() else {
        return false;
    };
    frame.at = position;
    let occurrence = frame.counts.get(position).copied().unwrap_or_default();
    if let Some(count) = frame.counts.get_mut(position) {
        *count = count.saturating_add(1);
    }
    match frame.nodes.get(position) {
        Some(Node::Segment(reference)) => {
            frame.items.push(Item::Segment(Placed {
                index,
                node: reference,
                occurrence,
            }));
            true
        }
        Some(Node::Group(group)) => {
            stack.push(Frame::instance(group, occurrence));
            let Some(inner) = stack.last().and_then(|frame| candidate(frame, id)) else {
                return false;
            };
            enter(stack, inner, id, index)
        }
        Some(Node::Placeholder(_)) | None => false,
    }
}

/// Closes the top frame into its parent.
fn close(stack: &mut Vec<Frame>, missing: &mut Vec<String>) {
    let Some(frame) = stack.pop() else {
        return;
    };
    frame.check(missing);
    let Some(group) = frame.group else {
        return;
    };
    if let Some(parent) = stack.last_mut() {
        parent.items.push(Item::Group(Instance {
            group,
            occurrence: frame.occurrence,
            items: frame.items,
        }));
    }
}

/// Whether `first` holds `id`.
fn starts(first: &[&'static str], id: &str) -> bool {
    first.contains(&id)
}

/// The segment ids a new instance of `group` can open with.
fn group_first(group: &'static Group) -> Vec<&'static str> {
    let mut first = Vec::new();
    for child in group.children {
        let (ids, required) = match child {
            Node::Segment(reference) => (vec![reference.segment.id], reference.cardinality.min > 0),
            Node::Group(inner) => (group_first(inner), inner.cardinality.min > 0),
            Node::Placeholder(_) => (Vec::new(), false),
        };
        first.extend(ids);
        if required && group.kind == GroupKind::Sequence {
            break;
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::{Delimiters, ErrorCode, LexError, Location, delimiters, lex};
    use crate::decode::Charset;

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

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn escape_sequences_decode_to_their_delimiters() -> Result<(), LexError> {
        let lexed = lex(
            "MSH|^~\\&|A\rNTE|||a\\F\\b\\S\\c\\E\\d\\X41\\",
            Charset::Ascii,
        )?;
        let note = lexed
            .message
            .segments()
            .get(1)
            .and_then(|segment| segment.text(3));
        assert_eq!(note, Some("a|b^c\\dA"));
        assert!(lexed.refusals.is_empty(), "{:?}", lexed.refusals);
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn an_undecoded_escape_sequence_is_refused_at_its_location() -> Result<(), LexError> {
        let lexed = lex("MSH|^~\\&|A\rNTE|||\\Zlocal\\", Charset::Ascii)?;
        let refusal = lexed.refusals.first();
        assert_eq!(
            refusal.map(|refusal| refusal.code),
            Some(ErrorCode::DataType)
        );
        assert_eq!(
            refusal.map(|refusal| refusal.location.erl('^')),
            Some(String::from("NTE^1^3^1^1^1"))
        );
        Ok(())
    }

    #[test]
    fn a_location_renders_its_known_erl_components() {
        let location = Location::segment("PID", 1).with_field(5);
        assert_eq!(location.erl('^'), "PID^1^5");
        assert_eq!(location.to_string(), "PID[1]-5");
    }
}
