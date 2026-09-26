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
//!
//! [`lex`]: lex::lex
//! [`group`]: grouping::group

pub mod grouping;
pub mod lex;
mod place;
mod required;
pub mod structure;

use core::fmt;

use hl7v2_types::model::{Group, Node, SegmentRef, Structure};

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
    /// parsed against them: the legacy tables carry no tree of the structure
    /// at that version.
    EarlierVersion {
        /// MSH-12.1 as the message wrote it.
        declared: String,
    },
    /// A field the selected definition marks required (`R`) is empty; the
    /// message is parsed and mapped all the same.
    MissingRequiredField {
        /// Where: the field.
        location: Location,
        /// The field's element id, for example `DG1.2` or `PID.5-patientName`.
        field: &'static str,
        /// The segment definition id, for example `DG1`.
        segment: &'static str,
        /// The version of the tables the structure comes from.
        version: &'static str,
    },
    /// The message is parsed against the tree and segment tables of the
    /// version named here, which MSH-12 selects ([`structure_for`]): its own
    /// version where the legacy tables carry the structure there, and v2.9.1
    /// otherwise. Not counted for a v2.9.1 message parsed against v2.9.1.
    ///
    /// [`structure_for`]: structure::structure_for
    VersionSelected {
        /// The structure id, for example `VXU_V04`.
        structure: &'static str,
        /// The version of the tables the tree comes from.
        version: &'static str,
        /// MSH-12.1 as the message wrote it.
        declared: Option<String>,
    },
    /// MSH-9.3 names a structure other than the one the message is grouped
    /// by: one the definitions lack, where the message definition of MSH-9.1
    /// and MSH-9.2 names the structure ([`structure_for`]).
    ///
    /// [`structure_for`]: structure::structure_for
    OtherStructure {
        /// MSH-9.3 as the message wrote it.
        declared: String,
        /// The id of the structure the message is grouped by.
        structure: &'static str,
    },
    /// The message is parsed against a structure the v2.9.1 definitions no
    /// longer carry, from the tables of an earlier version.
    WithdrawnStructure {
        /// The structure id, for example `ORM_O01`.
        structure: &'static str,
        /// The version of the tables the tree comes from.
        version: &'static str,
        /// The first version the sources carry without the structure.
        withdrawn_as_of: &'static str,
        /// MSH-12.1 as the message wrote it.
        declared: Option<String>,
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
            Self::VersionSelected { .. } => "version-selected",
            Self::MissingRequiredField { .. } => "missing-required-field",
            Self::OtherStructure { .. } => "other-structure",
            Self::WithdrawnStructure { .. } => "withdrawn-structure",
        }
    }
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

/// One open group instance while the segments are placed.
struct Frame {
    nodes: &'static [Node],
    group: Option<&'static Group>,
    occurrence: usize,
    at: usize,
    counts: Vec<usize>,
    items: Vec<Item>,
}
