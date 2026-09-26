// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering the v2 definitions to the tables `hl7v2-types` carries.
//!
//! No v2 model ships a snapshot, and every segment and message structure
//! specializes a base with no elements ([`crate::roots::V2_BASES`]), so the
//! differential is the complete element list and is read as the snapshot; a
//! definition carrying a snapshot, or a base carrying elements, is refused.
//!
//! Element ids are position-prefixed. A segment field is `SEG.n-key`
//! (`OBX.1-setId`): the segment id, the field position from 1, and a key. A
//! structure element is a dotted path from the structure id whose steps are
//! `n-NAME` for a segment or group at position `n` (`ORU_R01-A.5-PATIENT_RESULT`),
//! `choice-n-NAME` for the `n`th alternative of a choice group, and `segment`
//! and `group` for the two members of an `Hxx` slot. The name after the first
//! `-` that follows the position runs to the next `.` and may itself hold `-`
//! or `+` (`4-ANTI-MICROBIAL_DEVICE_DATA`). Positions run 1, 2, ... among
//! siblings, and every refusal names the file and the element id. A data type
//! component is `DT.n` (`CX.1`) with its type as a canonical URL. A message
//! definition is a constraint on the `Message` base whose `Message.structure`
//! is linked to a structure by name ([`Defect::StructureProfileName`]).
//!
//! Each defect the definitions carry is tolerated only in the files where it
//! was found ([`Defect::tolerated_in`]); the same defect anywhere else is
//! refused.

mod data_type;
mod extension;
mod message;
mod segment;
mod structure;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::roots::{V2_COMPLEX_DIR, V2_MESSAGE_DIR, V2_SEGMENT_DIR, V2RootSet};
use crate::v2::corpus::{Corpus, Sourced};
use crate::v2::definition::{Element, StructureDefinition};
use crate::v2::lower::data_type::lower_data_type;
use crate::v2::lower::message::lower_message;
use crate::v2::lower::segment::lower_segment;
use crate::v2::lower::structure::lower_structure;

/// The `optionality` extension on a segment field.
pub const OPTIONALITY: &str = "http://hl7.org/v2/StructureDefinition/optionality";

/// The `length` extension on a segment field.
pub const LENGTH: &str = "http://hl7.org/v2/StructureDefinition/length";

/// The `conformance-length` extension on a segment field.
pub const CONFORMANCE_LENGTH: &str = "http://hl7.org/v2/StructureDefinition/conformance-length";

/// The FHIR `standards-status` extension on a segment field.
pub const STANDARDS_STATUS: &str =
    "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status";

/// The `v2-segment-status` extension on a segment in a structure.
pub const SEGMENT_STATUS: &str = "http://hl7.org/v2/StructureDefinition/v2-segment-status";

/// The value set prefix of a table binding.
pub const TABLE_VALUE_SET: &str = "http://terminology.hl7.org/ValueSet/v2-";

/// The editorial placeholder some structures give a group in place of its name.
pub const PLACEHOLDER_GROUP_NAME: &str = "FIXME";

/// The type code of a segment group.
const BACKBONE: &str = "BackboneElement";

/// The canonical URL prefix of a v2 definition.
const V2_CANONICAL: &str = "http://hl7.org/v2/StructureDefinition/";

/// The HL7 v2 version the definitions hold.
// NOTE: HL7/v2ig V291_EXTRACTION_SUMMARY.md at the pinned commit describes the
// source of truth as extracted from the HL7 V2.9.1 documents.
pub const DEFINITIONS_VERSION: &str = "2.9.1";

/// A defect of the v2 definitions that lowering tolerates where it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Defect {
    /// `max` written as a JSON number; FHIR's `ElementDefinition.max` is a string.
    NumberMax,
    /// A `valueInteger` written as a JSON string.
    TextInteger,
    /// A `valueInteger` with no value (`null`).
    MissingInteger,
    /// A `conformance-length` with no `length`.
    ConformanceLengthWithoutLength,
    /// A `conformance-length` with no `noTruncate`.
    ConformanceLengthWithoutNoTruncate,
    /// The optionality `-`.
    DashOptionality,
    /// The segment status `d`.
    LowercaseStatus,
    /// An empty segment status.
    EmptyStatus,
    /// A field whose `min` exceeds its `max`.
    MinAboveMax,
    /// A data type code with no data type definition.
    UndefinedDataType,
    /// A message structure with no element under its root.
    EmptyStructure,
    /// A segment group whose name is the editorial placeholder
    /// [`PLACEHOLDER_GROUP_NAME`] in place of a group name.
    PlaceholderGroupName,
    /// An element member `defintion`, the misspelling of `definition`.
    MisspelledDefinition,
    /// A message definition whose `Message.structure` targets a profile URL
    /// no message structure carries, writing `-` for the `_` of the id.
    StructureProfileName,
    /// A message definition with no `Message.structure`.
    MessageWithoutStructure,
}

impl Defect {
    /// The files, or the directory, where the definitions carry the defect.
    #[must_use]
    pub const fn tolerated_in(self) -> &'static [&'static str] {
        match self {
            // NOTE: https://hl7.org/fhir/R5/elementdefinition.html types `max` as a
            // string; segment/segments/*.json write most of theirs as JSON numbers.
            Self::NumberMax => &[V2_SEGMENT_DIR],
            // NOTE: https://hl7.org/fhir/R5/json.html writes an integer as a JSON
            // number; these files write the conformance-length `length` as a string.
            Self::TextInteger => &[
                "segment/segments/BPX.json",
                "segment/segments/BTX.json",
                "segment/segments/CNS.json",
                "segment/segments/ECD.json",
                "segment/segments/EQP.json",
                "segment/segments/INV.json",
                "segment/segments/IVC.json",
                "segment/segments/NDS.json",
                "segment/segments/QPD.json",
                "segment/segments/SAC.json",
                "segment/segments/SID.json",
                "segment/segments/TCC.json",
            ],
            // NOTE: https://hl7.org/fhir/R5/json.html allows no null value; PM1.7,
            // TCC.15 and TQ2.6 give the length `max` as null.
            Self::MissingInteger => &[
                "segment/segments/PM1.json",
                "segment/segments/TCC.json",
                "segment/segments/TQ2.json",
            ],
            // NOTE: MSH.10, CP.3, CP.4, ERL.1 to ERL.6, MO.1 and MOP.2 carry a
            // conformance-length with `noTruncate` and no `length`.
            Self::ConformanceLengthWithoutLength => &[
                "data-type/complex/complex-data-types/cp.json",
                "data-type/complex/complex-data-types/erl.json",
                "data-type/complex/complex-data-types/mo.json",
                "data-type/complex/complex-data-types/mop.json",
                "segment/segments/MSH.json",
            ],
            // NOTE: segment/segments/DSP.json gives three conformance-lengths a
            // `length` and no `noTruncate`.
            Self::ConformanceLengthWithoutNoTruncate => &["segment/segments/DSP.json"],
            // NOTE: segment/segments/ROL.json writes `-` as the optionality of every field.
            Self::DashOptionality => &["segment/segments/ROL.json"],
            // NOTE: these files write the segment status `d` where the element short
            // reads "deprecated as of v2.9", and it is read as `D`.
            Self::LowercaseStatus => &[
                "message-structure/message_structures/PMU_B01-A.json",
                "message-structure/message_structures/PMU_B01-B.json",
                "message-structure/message_structures/PMU_B07.json",
                "message-structure/message_structures/RSP_K25.json",
            ],
            // NOTE: message-structure/message_structures/DFT_P03.json gives both ROL
            // entries an empty segment status, carried as no status.
            Self::EmptyStatus => &["message-structure/message_structures/DFT_P03.json"],
            // NOTE: segment/segments/EQU.json gives EQU.1 `min` 1 and `max` 0; both
            // are emitted as written.
            Self::MinAboveMax => &["segment/segments/EQU.json"],
            // NOTE: these files type one field `Varies`, whose only definition is
            // data-type/Varies.json outside the two type directories; the code stays.
            Self::UndefinedDataType => &[
                "segment/segments/MFA.json",
                "segment/segments/MFE.json",
                "segment/segments/OBX.json",
                "segment/segments/QPD.json",
                "segment/segments/RDT.json",
            ],
            // NOTE: message-structure/message_structures/QBP_Q21-F.json holds only
            // its root element, so the structure is emitted with no nodes.
            Self::EmptyStructure => &["message-structure/message_structures/QBP_Q21-F.json"],
            // NOTE: these files name one group with the editorial placeholder (MDM_T02-A.15),
            // which is also its short and definition; the name is emitted as written.
            Self::PlaceholderGroupName => &[
                "message-structure/message_structures/CSU_C09.json",
                "message-structure/message_structures/MDM_T02-A.json",
                "message-structure/message_structures/MDM_T02-B.json",
                "message-structure/message_structures/MDM_T02-C.json",
                "message-structure/message_structures/MDM_T02-D.json",
                "message-structure/message_structures/MDM_T02-E.json",
                "message-structure/message_structures/SRM_S01.json",
            ],
            // NOTE: https://hl7.org/fhir/R5/elementdefinition.html names the member
            // `definition`; the complex data type files write `defintion`, which is not read.
            Self::MisspelledDefinition => &[V2_COMPLEX_DIR],
            // NOTE: these files target http://hl7.org/fhir/StructureDefinition/MessageStructure/ORU-R01-A
            // for http://hl7.org/v2/StructureDefinition/ORU_R01-A, so the structure is linked by name.
            Self::StructureProfileName => &[V2_MESSAGE_DIR],
            // NOTE: meta-resources/message--message.json gives Message.structure `min` 1;
            // these files state none (32 withdrawn, EHC-E30 and EHC-E31 active).
            Self::MessageWithoutStructure => &[
                "message/messages/ADT-A18.json",
                "message/messages/ADT-A30.json",
                "message/messages/ADT-A34.json",
                "message/messages/ADT-A35.json",
                "message/messages/ADT-A36.json",
                "message/messages/ADT-A39.json",
                "message/messages/ADT-A46.json",
                "message/messages/ADT-A48.json",
                "message/messages/EHC-E30.json",
                "message/messages/EHC-E31.json",
                "message/messages/MFN-M01.json",
                "message/messages/MFN-M03.json",
                "message/messages/NMQ-N01.json",
                "message/messages/OUL-R21.json",
                "message/messages/PPT-PCL.json",
                "message/messages/PPV-PCA.json",
                "message/messages/PRR-PC5.json",
                "message/messages/PTR-PCF.json",
                "message/messages/QRY-A19.json",
                "message/messages/QRY-P04.json",
                "message/messages/QRY-PC4.json",
                "message/messages/QRY-PC9.json",
                "message/messages/QRY-PCE.json",
                "message/messages/QRY-PCK.json",
                "message/messages/QRY-R02.json",
                "message/messages/QRY-R04.json",
                "message/messages/QRY-T12.json",
                "message/messages/RQC-I05.json",
                "message/messages/RQC-I06.json",
                "message/messages/SQM-S25.json",
                "message/messages/SUR-P09.json",
                "message/messages/VXR-V03.json",
                "message/messages/VXX-V02.json",
                "message/messages/XQ-V01.json",
            ],
        }
    }

    /// Whether the definitions carry the defect in `file`.
    #[must_use]
    pub fn is_tolerated(self, file: &str) -> bool {
        self.tolerated_in().iter().any(|scope| {
            file == *scope
                || file
                    .strip_prefix(scope)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }
}

/// A failure while lowering the v2 definitions.
#[derive(Debug, thiserror::Error)]
pub enum LowerError {
    /// A definition or element breaks a rule the lowering relies on.
    #[error("{file}: {element}: {reason}")]
    Invalid {
        /// The file, relative to the definitions tree.
        file: String,
        /// The element id, or the definition id for a definition-level rule.
        element: String,
        /// What is wrong.
        reason: String,
    },
    /// A defect found outside the files where it is tolerated.
    #[error("{file}: {element}: {defect:?} outside the files that carry it")]
    Defect {
        /// The file, relative to the definitions tree.
        file: String,
        /// The element id.
        element: String,
        /// The defect.
        defect: Defect,
    },
    /// A number the lowering reads does not parse or does not fit.
    #[error("{file}: {element}: {value:?} is not a count the table holds")]
    Number {
        /// The file, relative to the definitions tree.
        file: String,
        /// The element id.
        element: String,
        /// The value as written.
        value: String,
        /// Why it does not parse or fit.
        #[source]
        source: NumberError,
    },
}

/// Why a number of the definitions is refused.
#[derive(Debug, thiserror::Error)]
pub enum NumberError {
    /// The text is not a decimal count.
    #[error(transparent)]
    Parse(#[from] std::num::ParseIntError),
    /// The count exceeds the emitted integer type.
    #[error(transparent)]
    Range(#[from] std::num::TryFromIntError),
}

fn number(
    sourced: &Input<'_>,
    element: &Element,
    value: &dyn std::fmt::Display,
) -> impl FnOnce(NumberError) -> LowerError {
    let file = sourced.file.to_owned();
    let element = element.id.clone();
    let value = value.to_string();
    move |source| LowerError::Number {
        file,
        element,
        value,
        source,
    }
}

/// A cardinality: `max` is `None` for `*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cardinality {
    /// The minimum.
    pub min: u32,
    /// The maximum, `None` when unbounded.
    pub max: Option<u32>,
}

/// An optionality code, as `hl7v2_types::model::Optionality` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Optionality {
    /// `R`.
    R,
    /// `RE`.
    Re,
    /// `O`.
    O,
    /// `C`.
    C,
    /// `C(first/second)`.
    Conditional(ConditionalCode, ConditionalCode),
    /// `X`.
    X,
    /// `B`.
    B,
    /// `W`.
    W,
    /// `NA`, which only the legacy tables write.
    Na,
    /// `-`.
    Unstated,
}

/// A code inside `C(first/second)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalCode {
    /// `R`.
    R,
    /// `RE`.
    Re,
    /// `O`.
    O,
    /// `X`.
    X,
}

/// The `length` extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Length {
    /// `min`.
    pub min: u32,
    /// `max`.
    pub max: Option<u32>,
}

/// The `conformance-length` extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConformanceLength {
    /// `length`.
    pub length: Option<u32>,
    /// `noTruncate`.
    pub no_truncate: Option<bool>,
}

/// A table binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The table number, for example `0125`.
    pub id: String,
    /// The value set URL.
    pub value_set: String,
}

/// A `standards-status` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardsStatus {
    /// `deprecated`.
    Deprecated,
    /// `withdrawn`.
    Withdrawn,
}

/// A `v2-segment-status` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStatus {
    /// `A`.
    A,
    /// `B`.
    B,
    /// `D`.
    D,
}

/// One segment field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The element id.
    pub id: String,
    /// The position from 1.
    pub position: u16,
    /// The element `short`.
    pub name: String,
    /// The data type code, `None` for a `W` field.
    pub data_type: Option<String>,
    /// The cardinality.
    pub cardinality: Cardinality,
    /// The optionality.
    pub optionality: Optionality,
    /// The length.
    pub length: Option<Length>,
    /// The conformance length.
    pub conformance_length: Option<ConformanceLength>,
    /// The table.
    pub table: Option<Table>,
    /// The standards status.
    pub standards_status: Option<StandardsStatus>,
}

/// One segment definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The definition id.
    pub id: String,
    /// The canonical URL, `None` for a legacy segment.
    pub url: Option<String>,
    /// The root element's `short`.
    pub name: String,
    /// The fields in position order.
    pub fields: Vec<Field>,
}

/// Whether a group's children follow each other or are alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// `n-NAME` children.
    Sequence,
    /// `choice-n-NAME` children.
    Choice,
}

/// One node of a structure's group tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A segment.
    Segment {
        /// The element id.
        id: String,
        /// The position among siblings.
        position: u16,
        /// The segment id.
        segment: String,
        /// The cardinality.
        cardinality: Cardinality,
        /// The segment status.
        status: Option<SegmentStatus>,
    },
    /// A group.
    Group {
        /// The element id.
        id: String,
        /// The position among siblings.
        position: u16,
        /// The group name.
        name: String,
        /// The cardinality.
        cardinality: Cardinality,
        /// Sequence or choice.
        kind: GroupKind,
        /// The children.
        children: Vec<Node>,
    },
    /// An `Hxx` slot.
    Placeholder {
        /// The element id.
        id: String,
        /// The position among siblings.
        position: u16,
        /// The cardinality.
        cardinality: Cardinality,
    },
}

/// One message structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    /// The definition id.
    pub id: String,
    /// The canonical URL, `None` for a legacy structure.
    pub url: Option<String>,
    /// The HL7 version of the tables it comes from.
    pub version: String,
    /// The first version the sources carry without it, for a legacy structure.
    pub withdrawn_as_of: Option<String>,
    /// The top-level nodes.
    pub nodes: Vec<Node>,
}

/// One component of a complex data type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// The element id, for example `CX.1`.
    pub id: String,
    /// The position from 1.
    pub position: u16,
    /// The element `short`.
    pub name: String,
    /// The data type code, `None` for a `W` component.
    pub data_type: Option<String>,
    /// The cardinality, `None` for a `W` component.
    pub cardinality: Option<Cardinality>,
    /// The optionality.
    pub optionality: Optionality,
    /// The length.
    pub length: Option<Length>,
    /// The conformance length.
    pub conformance_length: Option<ConformanceLength>,
    /// The table.
    pub table: Option<Table>,
}

/// One data type definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataType {
    /// The definition id, which is the data type code.
    pub code: String,
    /// The canonical URL.
    pub url: String,
    /// The root element's `short`.
    pub name: String,
    /// The components in position order, empty for a primitive.
    pub components: Vec<Component>,
}

/// A message definition's status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    /// `active`.
    Active,
    /// `withdrawn`.
    Withdrawn,
}

/// One message definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The definition id, for example `ORU-R01`.
    pub id: String,
    /// The canonical URL.
    pub url: String,
    /// The message code.
    pub code: String,
    /// The trigger event.
    pub event: String,
    /// The id of the message structure it names, when it names one.
    pub structure: Option<String>,
    /// The status.
    pub status: MessageStatus,
}

/// The lowered v2 tables: the structures, segments, data types and message
/// definitions of the root set.
#[derive(Debug, Clone)]
pub struct Model {
    /// The pinned v2ig commit.
    pub commit: String,
    /// Every message structure, by id.
    pub structures: BTreeMap<String, Structure>,
    /// Every segment definition, by id.
    pub segments: BTreeMap<String, Segment>,
    /// Every primitive and complex data type, by code.
    pub data_types: BTreeMap<String, DataType>,
    /// Every message definition, by id.
    pub messages: BTreeMap<String, Message>,
    /// Every tolerated defect the lowering met, with the file that carries it.
    pub tolerated: BTreeSet<(Defect, String)>,
}

impl Model {
    /// Lowers the root set of `corpus`: its structures and its segments.
    ///
    /// # Errors
    ///
    /// Returns [`LowerError::Invalid`] naming the file and element that break
    /// a rule the lowering relies on, and [`LowerError::Defect`] for a known
    /// defect outside the files where it is tolerated.
    pub fn lower(corpus: &Corpus, roots: &V2RootSet<'_>) -> Result<Self, LowerError> {
        let hits = RefCell::new(BTreeSet::new());
        for base in corpus.bases().values() {
            if base.definition.snapshot.is_some() || base.definition.differential.is_some() {
                return Err(invalid(
                    &Input::of(base, &hits),
                    &base.definition.id,
                    "a base carries elements, so a differential is not the complete element list",
                ));
            }
        }
        let mut structures = BTreeMap::new();
        let mut reached = BTreeMap::new();
        for (id, sourced) in &roots.structures {
            let structure = lower_structure(corpus, &Input::of(sourced, &hits), &mut reached)?;
            structures.insert((*id).to_owned(), structure);
        }
        for (id, sourced) in &roots.segments {
            reached.insert((*id).to_owned(), *sourced);
        }
        let mut segments = BTreeMap::new();
        for (id, sourced) in reached {
            segments.insert(id, lower_segment(corpus, &Input::of(sourced, &hits))?);
        }
        let mut data_types = BTreeMap::new();
        for (id, sourced) in &roots.data_types {
            let data_type = lower_data_type(corpus, &Input::of(sourced, &hits))?;
            data_types.insert((*id).to_owned(), data_type);
        }
        let mut messages = BTreeMap::new();
        for (id, sourced) in &roots.messages {
            let message = lower_message(sourced, &structures, &hits)?;
            messages.insert((*id).to_owned(), message);
        }
        Ok(Self {
            commit: corpus.commit().to_owned(),
            structures,
            segments,
            data_types,
            messages,
            tolerated: hits.into_inner(),
        })
    }

    /// The number of fields over every emitted segment.
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.segments
            .values()
            .map(|segment| segment.fields.len())
            .sum()
    }

    /// The number of components over every emitted data type.
    #[must_use]
    pub fn component_count(&self) -> usize {
        self.data_types
            .values()
            .map(|data_type| data_type.components.len())
            .sum()
    }

    /// The number of primitive data types: those with no components.
    #[must_use]
    pub fn primitive_count(&self) -> usize {
        self.data_types
            .values()
            .filter(|data_type| data_type.components.is_empty())
            .count()
    }
}

/// One definition being lowered, with the record of the defects met.
struct Input<'a> {
    file: &'a str,
    definition: &'a StructureDefinition,
    hits: &'a RefCell<BTreeSet<(Defect, String)>>,
}

impl<'a> Input<'a> {
    fn of(sourced: &'a Sourced, hits: &'a RefCell<BTreeSet<(Defect, String)>>) -> Self {
        Self {
            file: &sourced.file,
            definition: &sourced.definition,
            hits,
        }
    }
}

fn invalid(sourced: &Input<'_>, element: &str, reason: impl Into<String>) -> LowerError {
    invalid_in(sourced.file, element, reason)
}

fn invalid_in(file: &str, element: &str, reason: impl Into<String>) -> LowerError {
    LowerError::Invalid {
        file: file.to_owned(),
        element: element.to_owned(),
        reason: reason.into(),
    }
}

fn defect(sourced: &Input<'_>, element: &str, defect: Defect) -> Result<(), LowerError> {
    defect_in(sourced.file, sourced.hits, element, defect)
}

fn defect_in(
    file: &str,
    hits: &RefCell<BTreeSet<(Defect, String)>>,
    element: &str,
    defect: Defect,
) -> Result<(), LowerError> {
    if defect.is_tolerated(file) {
        hits.borrow_mut().insert((defect, file.to_owned()));
        Ok(())
    } else {
        Err(LowerError::Defect {
            file: file.to_owned(),
            element: element.to_owned(),
            defect,
        })
    }
}
