//! The shapes of the generated HL7 v2 tables.
//!
//! A [`Structure`] is one message structure of the v2 definitions: its
//! segment-group tree as [`Node`]s in definition order, each with the
//! cardinality the definition gives it. A [`Segment`] is one segment
//! definition: its fields by position, each with its data type, its
//! cardinality (the repetition), its optionality code, its lengths, and the
//! table its binding names. A [`DataType`] is one data type definition with
//! its components by position, and a [`Message`] is one message definition:
//! the message code and trigger event it is sent under and the structure it
//! names. Every value is the definition's own; a code is carried as the
//! definitions write it, and the element id of each field, component and
//! node is kept so a reader can find it in the source.

/// The cardinality of a field, a component, a segment in a group, or a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cardinality {
    /// The minimum number of occurrences.
    pub min: u32,
    /// The maximum number of occurrences.
    pub max: Max,
}

/// The upper bound of a [`Cardinality`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Max {
    /// At most this many occurrences.
    Bounded(u32),
    /// Any number of occurrences (`*`).
    Unbounded,
}

/// One message definition, for example `ORU-R01`.
#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    /// The definition id, for example `ORU-R01`.
    pub id: &'static str,
    /// The definition's canonical URL.
    pub url: &'static str,
    /// The message code (`MSH-9.1`), for example `ORU`.
    pub code: &'static str,
    /// The trigger event (`MSH-9.2`), for example `R01`: the definition's
    /// `triggerEvent` where it states one, and otherwise the event its id
    /// names after the code (`ACK-A01`).
    pub event: &'static str,
    /// The message structure the definition names, with its variant
    /// (`ORU_R01-A`); `None` for a definition that names none.
    pub structure: Option<&'static Structure>,
    /// The definition's status code.
    pub status: MessageStatus,
}

/// The status code of a [`Message`] definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    /// The code `active`.
    Active,
    /// The code `withdrawn`.
    Withdrawn,
}

/// One message structure, for example `ORU_R01-A`.
#[derive(Debug, PartialEq, Eq)]
pub struct Structure {
    /// The definition id, for example `ORU_R01-A`.
    pub id: &'static str,
    /// The definition's canonical URL.
    pub url: &'static str,
    /// The top-level segments and groups in definition order.
    pub nodes: &'static [Node],
}

/// One entry of a segment-group tree.
#[derive(Debug, PartialEq, Eq)]
pub enum Node {
    /// A segment at this position.
    Segment(SegmentRef),
    /// A segment group at this position.
    Group(Group),
    /// An open slot the definitions name `Hxx`: one segment, or a nested
    /// group of further such slots.
    Placeholder(Placeholder),
}

/// A segment at a position of a structure or a group.
#[derive(Debug, PartialEq, Eq)]
pub struct SegmentRef {
    /// The element id, for example `ORU_R01-A.1-MSH`.
    pub id: &'static str,
    /// The position among its siblings, from 1.
    pub position: u16,
    /// The segment definition.
    pub segment: &'static Segment,
    /// How often the segment occurs here.
    pub cardinality: Cardinality,
    /// The `v2-segment-status` code, when the definitions give one.
    pub status: Option<SegmentStatus>,
}

/// A segment group.
#[derive(Debug, PartialEq, Eq)]
pub struct Group {
    /// The element id, for example `ORU_R01-A.5-PATIENT_RESULT`.
    pub id: &'static str,
    /// The position among its siblings, from 1.
    pub position: u16,
    /// The group name, for example `PATIENT_RESULT`.
    pub name: &'static str,
    /// How often the group occurs.
    pub cardinality: Cardinality,
    /// Whether the children follow each other or are alternatives.
    pub kind: GroupKind,
    /// The children in definition order.
    pub children: &'static [Node],
}

/// How the children of a [`Group`] relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// The children follow each other in order (element ids `n-NAME`).
    Sequence,
    /// The children are alternatives (element ids `choice-n-NAME`).
    Choice,
}

/// An open `Hxx` slot of a structure.
#[derive(Debug, PartialEq, Eq)]
pub struct Placeholder {
    /// The element id, for example `QBP_Q15.5-Hxx`.
    pub id: &'static str,
    /// The position among its siblings, from 1.
    pub position: u16,
    /// How often the slot occurs.
    pub cardinality: Cardinality,
}

/// The `v2-segment-status` code of a segment in a structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStatus {
    /// The code `A`.
    A,
    /// The code `B`.
    B,
    /// The code `D`.
    D,
}

/// One segment definition, for example `OBX`.
#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    /// The definition id, for example `OBX`.
    pub id: &'static str,
    /// The definition's canonical URL.
    pub url: &'static str,
    /// The segment name, for example `Observation/Result`.
    pub name: &'static str,
    /// The fields in position order, from position 1.
    pub fields: &'static [Field],
}

/// One field of a segment.
#[derive(Debug, PartialEq, Eq)]
pub struct Field {
    /// The element id, for example `OBX.1-setId`.
    pub id: &'static str,
    /// The field position, from 1.
    pub position: u16,
    /// The field name, for example `Set ID – OBX`.
    pub name: &'static str,
    /// The data type; `None` for a field whose optionality is `W`, which
    /// the definitions leave untyped.
    pub data_type: Option<DataTypeRef>,
    /// The cardinality; its maximum is the field's repetition.
    pub cardinality: Cardinality,
    /// The optionality code.
    pub optionality: Optionality,
    /// The `length` extension, when present.
    pub length: Option<Length>,
    /// The `conformance-length` extension, when present.
    pub conformance_length: Option<ConformanceLength>,
    /// The table the binding names, when there is one.
    pub table: Option<Table>,
    /// The `structuredefinition-standards-status` code, when present.
    pub standards_status: Option<StandardsStatus>,
}

/// The data type a [`Field`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataTypeRef {
    /// A data type the definitions define.
    Defined(&'static DataType),
    /// A code no primitive or complex data type definition defines, for
    /// example `Varies`, carried as written.
    Undefined(&'static str),
}

impl DataTypeRef {
    /// Returns the data type code, for example `CWE` or `Varies`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Defined(data_type) => data_type.code,
            Self::Undefined(code) => code,
        }
    }
}

/// One data type definition, for example `CX`.
#[derive(Debug, PartialEq, Eq)]
pub struct DataType {
    /// The data type code, which is the definition id, for example `CX`.
    pub code: &'static str,
    /// The definition's canonical URL.
    pub url: &'static str,
    /// The data type name, for example `extended composite ID with check digit`.
    pub name: &'static str,
    /// The components in position order, from position 1; empty for a
    /// primitive data type.
    pub components: &'static [Component],
}

/// One component of a complex data type.
#[derive(Debug, PartialEq, Eq)]
pub struct Component {
    /// The element id, for example `CX.1`.
    pub id: &'static str,
    /// The component position, from 1.
    pub position: u16,
    /// The component name, for example `ID Number`.
    pub name: &'static str,
    /// The component's data type; `None` for a component whose
    /// optionality is `W`, which the definitions leave untyped.
    pub data_type: Option<&'static DataType>,
    /// The cardinality; `None` for a `W` component, which the definitions
    /// give none.
    pub cardinality: Option<Cardinality>,
    /// The optionality code.
    pub optionality: Optionality,
    /// The `length` extension, when present.
    pub length: Option<Length>,
    /// The `conformance-length` extension, when present.
    pub conformance_length: Option<ConformanceLength>,
    /// The table the binding names, when there is one.
    pub table: Option<Table>,
}

/// The `optionality` code of a field or a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Optionality {
    /// The code `R`.
    R,
    /// The code `RE`.
    Re,
    /// The code `O`.
    O,
    /// The code `C`.
    C,
    /// The code `C(first/second)`, for example `C(R/X)`.
    Conditional {
        /// The code before the slash.
        first: ConditionalCode,
        /// The code after the slash.
        second: ConditionalCode,
    },
    /// The code `X`.
    X,
    /// The code `B`.
    B,
    /// The code `W`.
    W,
    /// The definitions write `-` in place of a code.
    Unstated,
}

/// A code inside `C(first/second)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalCode {
    /// The code `R`.
    R,
    /// The code `RE`.
    Re,
    /// The code `O`.
    O,
    /// The code `X`.
    X,
}

/// The `length` extension of a field or a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Length {
    /// The `min` value.
    pub min: u32,
    /// The `max` value, `None` where the definitions give it no value.
    pub max: Option<u32>,
}

/// The `conformance-length` extension of a field or a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConformanceLength {
    /// The `length` value, `None` where the definitions give none.
    pub length: Option<u32>,
    /// The `noTruncate` value (written `1` or `0` on a segment field and
    /// `true` or `false` on a component), `None` where the definitions give
    /// none.
    pub no_truncate: Option<bool>,
}

/// The HL7 table a binding names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    /// The table number, for example `0125`.
    pub id: &'static str,
    /// The bound value set's canonical URL, for example
    /// `http://terminology.hl7.org/ValueSet/v2-0125`.
    pub value_set: &'static str,
}

/// The `structuredefinition-standards-status` code of a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardsStatus {
    /// The code `deprecated`.
    Deprecated,
    /// The code `withdrawn`.
    Withdrawn,
}
