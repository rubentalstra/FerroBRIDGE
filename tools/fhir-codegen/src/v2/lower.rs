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

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::fhir::{Derivation, StructureKind};
use crate::roots::{
    V2_COMPLEX_BASE, V2_COMPLEX_DIR, V2_MESSAGE_BASE, V2_MESSAGE_DIR, V2_PRIMITIVE_BASE,
    V2_SEGMENT_BASE, V2_SEGMENT_DIR, V2RootSet,
};
use crate::v2::corpus::{Corpus, Sourced, SourcedMessage};
use crate::v2::definition::{Element, Extension, MessageElement, Scalar, StructureDefinition};

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
    /// The canonical URL.
    pub url: String,
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
    /// The canonical URL.
    pub url: String,
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

/// The checks every segment and structure definition passes, returning its elements.
fn elements<'a>(sourced: &Input<'a>, base: &str) -> Result<&'a [Element], LowerError> {
    let definition = &sourced.definition;
    let id = definition.id.as_str();
    if definition.kind != StructureKind::Logical
        || definition.is_abstract
        || definition.derivation != Some(Derivation::Specialization)
        || definition.base_definition.as_deref() != Some(base)
    {
        return Err(invalid(
            sourced,
            id,
            format!("not a concrete logical specialization of {base}"),
        ));
    }
    if definition.url != format!("{V2_CANONICAL}{id}") {
        return Err(invalid(
            sourced,
            id,
            format!("canonical URL {} does not end in its id", definition.url),
        ));
    }
    // NOTE: https://hl7.org/fhir/R5/structuredefinition.html states a differential
    // relative to its base; no v2 file ships a snapshot and every base is
    // element-less, so the differential is read as the snapshot.
    if definition.snapshot.is_some() {
        return Err(invalid(
            sourced,
            id,
            "a snapshot is present; the lowering reads the differential as the snapshot",
        ));
    }
    let Some(differential) = &definition.differential else {
        return Err(invalid(sourced, id, "no differential"));
    };
    let elements = differential.element.as_slice();
    let Some((root, _)) = elements.split_first() else {
        return Err(invalid(sourced, id, "the differential has no elements"));
    };
    if root.id != id {
        return Err(invalid(
            sourced,
            &root.id,
            format!("the first element is not the root {id}"),
        ));
    }
    for element in elements {
        if element.path != element.id {
            return Err(invalid(
                sourced,
                &element.id,
                format!("path {} differs from the id", element.path),
            ));
        }
        if element.defintion.is_some() {
            defect(sourced, &element.id, Defect::MisspelledDefinition)?;
        }
    }
    Ok(elements)
}

fn cardinality(sourced: &Input<'_>, element: &Element) -> Result<Cardinality, LowerError> {
    let Some(min) = element.min else {
        return Err(invalid(sourced, &element.id, "no min"));
    };
    let max = match &element.max {
        None => return Err(invalid(sourced, &element.id, "no max")),
        Some(Scalar::Text(text)) if text == "*" => None,
        Some(Scalar::Text(text)) => Some(
            text.parse::<u32>()
                .map_err(NumberError::from)
                .map_err(number(sourced, element, text))?,
        ),
        Some(Scalar::Integer(count)) => {
            defect(sourced, &element.id, Defect::NumberMax)?;
            Some(
                u32::try_from(*count)
                    .map_err(NumberError::from)
                    .map_err(number(sourced, element, count))?,
            )
        }
    };
    if let Some(max) = max
        && min > max
    {
        defect(sourced, &element.id, Defect::MinAboveMax)?;
    }
    Ok(Cardinality { min, max })
}

fn position(sourced: &Input<'_>, element: &str, digits: &str) -> Result<u16, LowerError> {
    let valid = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    let parsed = if valid {
        digits.parse::<u16>().ok()
    } else {
        None
    };
    parsed.ok_or_else(|| {
        invalid(
            sourced,
            element,
            format!("position {digits:?} is not a number from 1"),
        )
    })
}

/// The single type code of an element, or `None` when it has no type.
fn single_type<'a>(
    sourced: &Input<'_>,
    element: &'a Element,
) -> Result<Option<&'a str>, LowerError> {
    match element.types.as_deref() {
        None => Ok(None),
        Some([only]) => Ok(Some(only.code.as_str())),
        Some(_) => Err(invalid(sourced, &element.id, "not exactly one type")),
    }
}

fn lower_segment(corpus: &Corpus, sourced: &Input<'_>) -> Result<Segment, LowerError> {
    let elements = elements(sourced, V2_SEGMENT_BASE)?;
    let id = sourced.definition.id.as_str();
    let Some((root, fields)) = elements.split_first() else {
        return Err(invalid(sourced, id, "the differential has no elements"));
    };
    let name = root
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &root.id, "the root element has no short name"))?;
    let mut lowered = Vec::with_capacity(fields.len());
    for (index, element) in fields.iter().enumerate() {
        lowered.push(lower_field(corpus, sourced, element, index)?);
    }
    Ok(Segment {
        id: id.to_owned(),
        url: sourced.definition.url.clone(),
        name,
        fields: lowered,
    })
}

/// The data type code of a field: absent exactly when its optionality is `W`.
fn data_type(
    corpus: &Corpus,
    sourced: &Input<'_>,
    element: &Element,
    optionality: Optionality,
) -> Result<Option<String>, LowerError> {
    match (single_type(sourced, element)?, optionality) {
        (None, Optionality::W) => Ok(None),
        (None, _) => Err(invalid(
            sourced,
            &element.id,
            "an untyped field whose optionality is not W",
        )),
        (Some(_), Optionality::W) => Err(invalid(
            sourced,
            &element.id,
            "a typed field whose optionality is W",
        )),
        (Some(code), _) => {
            if !corpus
                .data_types()
                .contains_key(&format!("{V2_CANONICAL}{code}"))
            {
                defect(sourced, &element.id, Defect::UndefinedDataType)?;
            }
            Ok(Some(code.to_owned()))
        }
    }
}

/// The table a field's binding names, from its `v2-NNNN` value set URL.
fn table(sourced: &Input<'_>, element: &Element) -> Result<Option<Table>, LowerError> {
    let Some(binding) = &element.binding else {
        return Ok(None);
    };
    if binding.strength != "required" {
        return Err(invalid(
            sourced,
            &element.id,
            format!("binding strength {}", binding.strength),
        ));
    }
    let number = binding
        .value_set
        .strip_prefix(TABLE_VALUE_SET)
        .filter(|number| number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| {
            invalid(
                sourced,
                &element.id,
                format!("binding {} names no v2 table", binding.value_set),
            )
        })?;
    Ok(Some(Table {
        id: number.to_owned(),
        value_set: binding.value_set.clone(),
    }))
}

fn lower_field(
    corpus: &Corpus,
    sourced: &Input<'_>,
    element: &Element,
    index: usize,
) -> Result<Field, LowerError> {
    let segment = sourced.definition.id.as_str();
    let step = element
        .id
        .strip_prefix(segment)
        .and_then(|rest| rest.strip_prefix('.'))
        .ok_or_else(|| invalid(sourced, &element.id, format!("not a field of {segment}")))?;
    let Some((digits, key)) = step.split_once('-') else {
        return Err(invalid(sourced, &element.id, "a field id is SEG.n-key"));
    };
    if key.is_empty() || key.contains('.') {
        return Err(invalid(sourced, &element.id, "a field id is SEG.n-key"));
    }
    let position = position(sourced, &element.id, digits)?;
    if usize::from(position) != index + 1 {
        return Err(invalid(
            sourced,
            &element.id,
            format!("field {} out of order", index + 1),
        ));
    }
    if element.content_reference.is_some() {
        return Err(invalid(
            sourced,
            &element.id,
            "a field carries a contentReference",
        ));
    }
    let name = element
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &element.id, "no short name"))?;
    let cardinality = cardinality(sourced, element)?;
    let extensions = element_extensions(sourced, element, true)?;
    let data_type = data_type(corpus, sourced, element, extensions.optionality)?;
    let table = table(sourced, element)?;
    Ok(Field {
        id: element.id.clone(),
        position,
        name,
        data_type,
        cardinality,
        optionality: extensions.optionality,
        length: extensions.length,
        conformance_length: extensions.conformance_length,
        table,
        standards_status: extensions.standards_status,
    })
}

/// The v2 extensions of a field or a component.
struct Extensions {
    optionality: Optionality,
    length: Option<Length>,
    conformance_length: Option<ConformanceLength>,
    standards_status: Option<StandardsStatus>,
}

/// Reads the extensions of a field (`standards_status` allowed) or a
/// component, each at most once and the optionality always.
fn element_extensions(
    sourced: &Input<'_>,
    element: &Element,
    standards_status_allowed: bool,
) -> Result<Extensions, LowerError> {
    let mut optionality = None;
    let mut length = None;
    let mut conformance_length = None;
    let mut standards_status = None;
    for extension in &element.extension {
        let slot_taken = match extension.url.as_str() {
            OPTIONALITY => optionality
                .replace(lower_optionality(sourced, element, extension)?)
                .is_some(),
            LENGTH => length
                .replace(lower_length(sourced, element, extension)?)
                .is_some(),
            CONFORMANCE_LENGTH => conformance_length
                .replace(lower_conformance_length(sourced, element, extension)?)
                .is_some(),
            STANDARDS_STATUS if standards_status_allowed => standards_status
                .replace(lower_standards_status(sourced, element, extension)?)
                .is_some(),
            other => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("unknown extension {other}"),
                ));
            }
        };
        if slot_taken {
            return Err(invalid(
                sourced,
                &element.id,
                format!("extension {} given twice", extension.url),
            ));
        }
    }
    let optionality =
        optionality.ok_or_else(|| invalid(sourced, &element.id, "no optionality extension"))?;
    Ok(Extensions {
        optionality,
        length,
        conformance_length,
        standards_status,
    })
}

fn value_code<'a>(
    sourced: &Input<'_>,
    element: &Element,
    extension: &'a Extension,
) -> Result<&'a str, LowerError> {
    if extension.value_integer.is_some()
        || extension.value_boolean.is_some()
        || !extension.extension.is_empty()
    {
        return Err(invalid(
            sourced,
            &element.id,
            format!("{} carries more than a code", extension.url),
        ));
    }
    extension.value_code.as_deref().ok_or_else(|| {
        invalid(
            sourced,
            &element.id,
            format!("{} carries no code", extension.url),
        )
    })
}

fn lower_optionality(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<Optionality, LowerError> {
    let code = value_code(sourced, element, extension)?;
    let simple = match code {
        "R" => Some(Optionality::R),
        "RE" => Some(Optionality::Re),
        "O" => Some(Optionality::O),
        "C" => Some(Optionality::C),
        "X" => Some(Optionality::X),
        "B" => Some(Optionality::B),
        "W" => Some(Optionality::W),
        "-" => {
            defect(sourced, &element.id, Defect::DashOptionality)?;
            Some(Optionality::Unstated)
        }
        _ => None,
    };
    if let Some(simple) = simple {
        return Ok(simple);
    }
    let conditional = code
        .strip_prefix("C(")
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|inner| inner.split_once('/'));
    let Some((first, second)) = conditional else {
        return Err(invalid(
            sourced,
            &element.id,
            format!("optionality {code:?}"),
        ));
    };
    let inner = |text: &str| match text {
        "R" => Ok(ConditionalCode::R),
        "RE" => Ok(ConditionalCode::Re),
        "O" => Ok(ConditionalCode::O),
        "X" => Ok(ConditionalCode::X),
        _ => Err(invalid(
            sourced,
            &element.id,
            format!("optionality {code:?}"),
        )),
    };
    Ok(Optionality::Conditional(inner(first)?, inner(second)?))
}

/// The value of one nested extension of a complex extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nested {
    /// A `valueInteger`, `None` where it is written `null`.
    Integer(Option<u64>),
    /// A `valueBoolean`.
    Boolean(bool),
}

/// The nested `valueInteger`s and `valueBoolean`s of a complex extension, by nested URL.
fn integers(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
    names: &[&str],
) -> Result<BTreeMap<String, Nested>, LowerError> {
    if extension.value_code.is_some()
        || extension.value_integer.is_some()
        || extension.value_boolean.is_some()
    {
        return Err(invalid(
            sourced,
            &element.id,
            format!("{} carries a value of its own", extension.url),
        ));
    }
    let mut values = BTreeMap::new();
    for nested in &extension.extension {
        if !names.contains(&nested.url.as_str())
            || nested.value_code.is_some()
            || !nested.extension.is_empty()
        {
            return Err(invalid(
                sourced,
                &element.id,
                format!("{} carries {:?}", extension.url, nested.url),
            ));
        }
        let value = match (&nested.value_integer, nested.value_boolean) {
            (None, Some(flag)) => Nested::Boolean(flag),
            (Some(_), Some(_)) => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("{} carries an integer and a boolean", nested.url),
                ));
            }
            (None, None) => {
                defect(sourced, &element.id, Defect::MissingInteger)?;
                Nested::Integer(None)
            }
            (Some(Scalar::Integer(number)), None) => Nested::Integer(Some(*number)),
            (Some(Scalar::Text(text)), None) => {
                defect(sourced, &element.id, Defect::TextInteger)?;
                Nested::Integer(Some(
                    text.parse::<u64>()
                        .map_err(NumberError::from)
                        .map_err(number(sourced, element, text))?,
                ))
            }
        };
        if values.insert(nested.url.clone(), value).is_some() {
            return Err(invalid(
                sourced,
                &element.id,
                format!("{} given twice", nested.url),
            ));
        }
    }
    Ok(values)
}

fn to_u32(sourced: &Input<'_>, element: &Element, value: u64) -> Result<u32, LowerError> {
    u32::try_from(value)
        .map_err(NumberError::from)
        .map_err(number(sourced, element, &value))
}

/// An integer nested in a complex extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NestedInteger {
    /// No nested extension of that name.
    Absent,
    /// A nested extension whose value is `null`.
    Null,
    /// A nested extension with a value.
    Value(u64),
}

/// The integer nested under `name`, refusing a boolean there.
fn nested_integer(
    sourced: &Input<'_>,
    element: &Element,
    values: &BTreeMap<String, Nested>,
    name: &str,
) -> Result<NestedInteger, LowerError> {
    match values.get(name) {
        None => Ok(NestedInteger::Absent),
        Some(Nested::Integer(None)) => Ok(NestedInteger::Null),
        Some(Nested::Integer(Some(value))) => Ok(NestedInteger::Value(*value)),
        Some(Nested::Boolean(_)) => Err(invalid(
            sourced,
            &element.id,
            format!("{name} is a boolean, not an integer"),
        )),
    }
}

fn lower_length(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<Length, LowerError> {
    let values = integers(sourced, element, extension, &["min", "max"])?;
    let NestedInteger::Value(min) = nested_integer(sourced, element, &values, "min")? else {
        return Err(invalid(sourced, &element.id, "a length without min"));
    };
    let max = match nested_integer(sourced, element, &values, "max")? {
        NestedInteger::Absent => {
            return Err(invalid(sourced, &element.id, "a length without max"));
        }
        NestedInteger::Null => None,
        NestedInteger::Value(max) => Some(max),
    };
    Ok(Length {
        min: to_u32(sourced, element, min)?,
        max: max.map(|max| to_u32(sourced, element, max)).transpose()?,
    })
}

fn lower_conformance_length(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<ConformanceLength, LowerError> {
    let values = integers(sourced, element, extension, &["length", "noTruncate"])?;
    let length = match nested_integer(sourced, element, &values, "length")? {
        NestedInteger::Value(length) => Some(to_u32(sourced, element, length)?),
        NestedInteger::Null => {
            return Err(invalid(
                sourced,
                &element.id,
                "a conformance length without a value",
            ));
        }
        NestedInteger::Absent => {
            defect(sourced, &element.id, Defect::ConformanceLengthWithoutLength)?;
            None
        }
    };
    // NOTE: no specification governs this: our own design; no definition of the
    // conformance-length extension is published, and fields write 1/0 where components write true/false.
    let no_truncate = match values.get("noTruncate") {
        Some(Nested::Integer(Some(0)) | Nested::Boolean(false)) => Some(false),
        Some(Nested::Integer(Some(1)) | Nested::Boolean(true)) => Some(true),
        Some(other) => {
            return Err(invalid(
                sourced,
                &element.id,
                format!("noTruncate {other:?} is neither 0 nor 1"),
            ));
        }
        None => {
            defect(
                sourced,
                &element.id,
                Defect::ConformanceLengthWithoutNoTruncate,
            )?;
            None
        }
    };
    Ok(ConformanceLength {
        length,
        no_truncate,
    })
}

fn lower_standards_status(
    sourced: &Input<'_>,
    element: &Element,
    extension: &Extension,
) -> Result<StandardsStatus, LowerError> {
    match value_code(sourced, element, extension)? {
        "deprecated" => Ok(StandardsStatus::Deprecated),
        "withdrawn" => Ok(StandardsStatus::Withdrawn),
        other => Err(invalid(
            sourced,
            &element.id,
            format!("standards status {other:?}"),
        )),
    }
}

/// One step of a structure element id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step<'a> {
    /// `n-NAME`.
    Numbered(u16, &'a str),
    /// `choice-n-NAME`.
    Choice(u16, &'a str),
    /// `segment`, the single-segment member of an `Hxx` slot.
    SlotSegment,
    /// `group`, the nested-group member of an `Hxx` slot.
    SlotGroup,
}

fn parse_step<'a>(
    sourced: &Input<'_>,
    element: &str,
    step: &'a str,
) -> Result<Step<'a>, LowerError> {
    match step {
        "segment" => return Ok(Step::SlotSegment),
        "group" => return Ok(Step::SlotGroup),
        _ => {}
    }
    let (choice, rest) = match step.strip_prefix("choice-") {
        Some(rest) => (true, rest),
        None => (false, step),
    };
    let Some((digits, name)) = rest.split_once('-') else {
        return Err(invalid(
            sourced,
            element,
            format!("step {step:?} is not n-NAME"),
        ));
    };
    if name.is_empty() {
        return Err(invalid(
            sourced,
            element,
            format!("step {step:?} has no name"),
        ));
    }
    let position = position(sourced, element, digits)?;
    Ok(if choice {
        Step::Choice(position, name)
    } else {
        Step::Numbered(position, name)
    })
}

/// The children of each element id, in definition order.
type Children<'a> = BTreeMap<&'a str, Vec<&'a Element>>;

fn lower_structure<'a>(
    corpus: &'a Corpus,
    sourced: &Input<'_>,
    reached: &mut BTreeMap<String, &'a Sourced>,
) -> Result<Structure, LowerError> {
    let elements = elements(sourced, crate::roots::V2_STRUCTURE_BASE)?;
    let id = sourced.definition.id.as_str();
    let mut children: Children<'_> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    seen.insert(id);
    for element in elements.iter().skip(1) {
        let Some((parent, _)) = element.id.rsplit_once('.') else {
            return Err(invalid(
                sourced,
                &element.id,
                "an element outside the structure",
            ));
        };
        if !seen.contains(parent) {
            return Err(invalid(
                sourced,
                &element.id,
                format!("its parent {parent} does not come before it"),
            ));
        }
        if !seen.insert(element.id.as_str()) {
            return Err(invalid(sourced, &element.id, "the id is given twice"));
        }
        children.entry(parent).or_default().push(element);
    }
    let nodes = if children.is_empty() {
        defect(sourced, id, Defect::EmptyStructure)?;
        Vec::new()
    } else {
        let (kind, nodes) = lower_children(corpus, sourced, &children, id, reached)?;
        if kind != GroupKind::Sequence {
            return Err(invalid(sourced, id, "the structure root is a choice"));
        }
        nodes
    };
    Ok(Structure {
        id: id.to_owned(),
        url: sourced.definition.url.clone(),
        nodes,
    })
}

fn lower_children<'a>(
    corpus: &'a Corpus,
    sourced: &Input<'_>,
    children: &Children<'_>,
    parent: &str,
    reached: &mut BTreeMap<String, &'a Sourced>,
) -> Result<(GroupKind, Vec<Node>), LowerError> {
    let Some(elements) = children.get(parent) else {
        return Err(invalid(
            sourced,
            parent,
            "a structure or group with no children",
        ));
    };
    let mut kind = None;
    let mut nodes = Vec::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
        let last = element
            .id
            .rsplit_once('.')
            .map_or(element.id.as_str(), |(_, last)| last);
        let (step_kind, position, name) = match parse_step(sourced, &element.id, last)? {
            Step::Numbered(position, name) => (GroupKind::Sequence, position, name),
            Step::Choice(position, name) => (GroupKind::Choice, position, name),
            Step::SlotSegment | Step::SlotGroup => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    "an Hxx member outside an Hxx slot",
                ));
            }
        };
        if *kind.get_or_insert(step_kind) != step_kind {
            return Err(invalid(
                sourced,
                &element.id,
                "sequence and choice steps mixed among siblings",
            ));
        }
        if usize::from(position) != index + 1 {
            return Err(invalid(
                sourced,
                &element.id,
                format!("position {position} where {} belongs", index + 1),
            ));
        }
        nodes.push(lower_node(
            corpus, sourced, children, element, position, name, reached,
        )?);
    }
    Ok((kind.unwrap_or(GroupKind::Sequence), nodes))
}

fn lower_node<'a>(
    corpus: &'a Corpus,
    sourced: &Input<'_>,
    children: &Children<'_>,
    element: &Element,
    position: u16,
    name: &str,
    reached: &mut BTreeMap<String, &'a Sourced>,
) -> Result<Node, LowerError> {
    if element.content_reference.is_some() {
        return Err(invalid(
            sourced,
            &element.id,
            "a contentReference outside an Hxx slot",
        ));
    }
    let cardinality = cardinality(sourced, element)?;
    let status = segment_status(sourced, element)?;
    let Some(code) = single_type(sourced, element)? else {
        return Err(invalid(
            sourced,
            &element.id,
            "an untyped structure element",
        ));
    };
    if code == BACKBONE {
        if status.is_some() {
            return Err(invalid(
                sourced,
                &element.id,
                "a group carries a segment status",
            ));
        }
        if let Some(slot) = placeholder(sourced, children, element)? {
            return Ok(Node::Placeholder {
                id: element.id.clone(),
                position,
                cardinality: slot,
            });
        }
        if name == PLACEHOLDER_GROUP_NAME {
            defect(sourced, &element.id, Defect::PlaceholderGroupName)?;
        }
        let (kind, nodes) = lower_children(corpus, sourced, children, &element.id, reached)?;
        return Ok(Node::Group {
            id: element.id.clone(),
            position,
            name: name.to_owned(),
            cardinality,
            kind,
            children: nodes,
        });
    }
    if children.contains_key(element.id.as_str()) {
        return Err(invalid(sourced, &element.id, "a segment with children"));
    }
    let Some(target) = corpus.segments().get(code) else {
        return Err(invalid(
            sourced,
            &element.id,
            format!("type {code} is no segment definition"),
        ));
    };
    let segment = target.definition.id.clone();
    if segment != name {
        return Err(invalid(
            sourced,
            &element.id,
            format!("step name {name} names another segment than {code}"),
        ));
    }
    reached.insert(segment.clone(), target);
    Ok(Node::Segment {
        id: element.id.clone(),
        position,
        segment,
        cardinality,
        status,
    })
}

/// The cardinality of `element` when it is an `Hxx` slot: a group whose
/// children are the `segment` and `group` members.
fn placeholder(
    sourced: &Input<'_>,
    children: &Children<'_>,
    element: &Element,
) -> Result<Option<Cardinality>, LowerError> {
    let Some(members) = children.get(element.id.as_str()) else {
        return Ok(None);
    };
    let steps: Vec<&str> = members
        .iter()
        .map(|member| member.id.rsplit_once('.').map_or("", |(_, last)| last))
        .collect();
    if !steps
        .iter()
        .any(|step| *step == "segment" || *step == "group")
    {
        return Ok(None);
    }
    let name = element.id.rsplit_once('.').map_or("", |(_, last)| last);
    let named = name.ends_with("-Hxx");
    let [segment, group] = members.as_slice() else {
        return Err(invalid(
            sourced,
            &element.id,
            "an Hxx slot is exactly a segment and a group member",
        ));
    };
    let one = Cardinality {
        min: 0,
        max: Some(1),
    };
    let segment_ok = steps.first() == Some(&"segment")
        && single_type(sourced, segment)? == Some(V2_SEGMENT_BASE)
        && cardinality(sourced, segment)? == one
        && segment.extension.is_empty()
        && !children.contains_key(segment.id.as_str());
    let group_ok = steps.get(1) == Some(&"group")
        && group.types.is_none()
        && group.content_reference.as_deref() == Some(format!("#{}", element.id).as_str())
        && cardinality(sourced, group)? == one
        && group.extension.is_empty()
        && !children.contains_key(group.id.as_str());
    if !(named && segment_ok && group_ok) {
        return Err(invalid(
            sourced,
            &element.id,
            "an Hxx slot is exactly a segment and a group member",
        ));
    }
    cardinality(sourced, element).map(Some)
}

fn segment_status(
    sourced: &Input<'_>,
    element: &Element,
) -> Result<Option<SegmentStatus>, LowerError> {
    let mut status = None;
    for extension in &element.extension {
        if extension.url != SEGMENT_STATUS {
            return Err(invalid(
                sourced,
                &element.id,
                format!("unknown extension {}", extension.url),
            ));
        }
        if status.is_some() {
            return Err(invalid(sourced, &element.id, "segment status given twice"));
        }
        let code = value_code(sourced, element, extension)?;
        status = Some(match code {
            "A" => Some(SegmentStatus::A),
            "B" => Some(SegmentStatus::B),
            "D" => Some(SegmentStatus::D),
            "d" => {
                defect(sourced, &element.id, Defect::LowercaseStatus)?;
                Some(SegmentStatus::D)
            }
            "" => {
                defect(sourced, &element.id, Defect::EmptyStatus)?;
                None
            }
            other => {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("segment status {other:?}"),
                ));
            }
        });
    }
    Ok(status.flatten())
}

fn lower_data_type(corpus: &Corpus, sourced: &Input<'_>) -> Result<DataType, LowerError> {
    let definition = sourced.definition;
    let id = definition.id.as_str();
    let primitive = definition.base_definition.as_deref() == Some(V2_PRIMITIVE_BASE);
    let base = if primitive {
        V2_PRIMITIVE_BASE
    } else {
        V2_COMPLEX_BASE
    };
    let elements = elements(sourced, base)?;
    if definition.type_name != definition.url {
        return Err(invalid(
            sourced,
            id,
            format!("type {} is not the canonical URL", definition.type_name),
        ));
    }
    let Some((root, components)) = elements.split_first() else {
        return Err(invalid(sourced, id, "the differential has no elements"));
    };
    if root.types.is_some() || root.binding.is_some() || !root.extension.is_empty() {
        return Err(invalid(
            sourced,
            &root.id,
            "the root element carries a type, a binding or an extension",
        ));
    }
    let name = root
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &root.id, "the root element has no short name"))?;
    match (primitive, components.is_empty()) {
        (true, false) => {
            return Err(invalid(
                sourced,
                id,
                "a primitive data type with components",
            ));
        }
        (false, true) => {
            return Err(invalid(
                sourced,
                id,
                "a complex data type with no components",
            ));
        }
        _ => {}
    }
    let mut lowered = Vec::with_capacity(components.len());
    for (index, element) in components.iter().enumerate() {
        lowered.push(lower_component(corpus, sourced, element, index)?);
    }
    Ok(DataType {
        code: id.to_owned(),
        url: definition.url.clone(),
        name,
        components: lowered,
    })
}

fn lower_component(
    corpus: &Corpus,
    sourced: &Input<'_>,
    element: &Element,
    index: usize,
) -> Result<Component, LowerError> {
    let data_type = sourced.definition.id.as_str();
    let digits = element
        .id
        .strip_prefix(data_type)
        .and_then(|rest| rest.strip_prefix('.'))
        .ok_or_else(|| {
            invalid(
                sourced,
                &element.id,
                format!("not a component of {data_type}"),
            )
        })?;
    let position = position(sourced, &element.id, digits)?;
    if usize::from(position) != index + 1 {
        return Err(invalid(
            sourced,
            &element.id,
            format!("component {} out of order", index + 1),
        ));
    }
    if element.content_reference.is_some() {
        return Err(invalid(
            sourced,
            &element.id,
            "a component carries a contentReference",
        ));
    }
    let name = element
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &element.id, "no short name"))?;
    let extensions = element_extensions(sourced, element, false)?;
    let withdrawn = extensions.optionality == Optionality::W;
    let code = match (single_type(sourced, element)?, withdrawn) {
        (None, true) => None,
        (None, false) => {
            return Err(invalid(
                sourced,
                &element.id,
                "an untyped component whose optionality is not W",
            ));
        }
        (Some(_), true) => {
            return Err(invalid(
                sourced,
                &element.id,
                "a typed component whose optionality is W",
            ));
        }
        (Some(url), false) => {
            let Some(code) = url.strip_prefix(V2_CANONICAL) else {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("type {url} is not a v2 canonical URL"),
                ));
            };
            if !corpus.data_types().contains_key(url) {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("type {url} is no data type definition"),
                ));
            }
            Some(code.to_owned())
        }
    };
    let cardinality = if withdrawn && element.min.is_none() && element.max.is_none() {
        None
    } else {
        Some(cardinality(sourced, element)?)
    };
    let table = table(sourced, element)?;
    Ok(Component {
        id: element.id.clone(),
        position,
        name,
        data_type: code,
        cardinality,
        optionality: extensions.optionality,
        length: extensions.length,
        conformance_length: extensions.conformance_length,
        table,
    })
}

// The element ids a message definition constrains.
const MESSAGE_TYPE: &str = "Message.messageType";
const TRIGGER_EVENT: &str = "Message.triggerEvent";
const MESSAGE_STRUCTURE: &str = "Message.structure";
const MESSAGE_STATUS: &str = "Message.status";
const ACKNOWLEDGEMENTS: [&str; 3] = [
    "Message.acknowledgementChoreography.originalModeResponse",
    "Message.acknowledgementChoreography.enhancedModeImmediateResponse",
    "Message.acknowledgementChoreography.enhancedModeApplicationResponse",
];
/// The profile URL prefix `Message.structure` targets.
const STRUCTURE_PROFILE: &str = "http://hl7.org/fhir/StructureDefinition/MessageStructure/";

fn lower_message(
    sourced: &SourcedMessage,
    structures: &BTreeMap<String, Structure>,
    hits: &RefCell<BTreeSet<(Defect, String)>>,
) -> Result<Message, LowerError> {
    let file = sourced.file.as_str();
    let definition = &sourced.definition;
    let id = definition.id.as_str();
    if definition.kind != StructureKind::Logical
        || definition.is_abstract
        || definition.type_name != "Message"
        || definition.derivation != Some(Derivation::Constraint)
        || definition.base_definition.as_deref() != Some(V2_MESSAGE_BASE)
    {
        return Err(invalid_in(
            file,
            id,
            format!("not a concrete logical constraint on {V2_MESSAGE_BASE}"),
        ));
    }
    if definition.url != format!("{V2_CANONICAL}Message/{id}") {
        return Err(invalid_in(
            file,
            id,
            format!("canonical URL {} does not end in its id", definition.url),
        ));
    }
    let Some(differential) = &definition.differential else {
        return Err(invalid_in(file, id, "no differential"));
    };
    let (codes, structure_element) = message_elements(file, &differential.element)?;
    let Some(code) = codes.get(MESSAGE_TYPE).copied() else {
        return Err(invalid_in(file, id, "no Message.messageType"));
    };
    let Some(id_event) = id
        .strip_prefix(code)
        .and_then(|rest| rest.strip_prefix('-'))
        .filter(|event| !event.is_empty())
    else {
        return Err(invalid_in(
            file,
            id,
            format!("the id is not {code}-<event>"),
        ));
    };
    // NOTE: no specification governs this: our own design; message--message.json gives
    // Message.triggerEvent `min` 0, and a definition that states none names its event in its id.
    let event = match codes.get(TRIGGER_EVENT) {
        Some(stated) if *stated != id_event => {
            return Err(invalid_in(
                file,
                TRIGGER_EVENT,
                format!("the trigger event {stated} is not the {id_event} the id names"),
            ));
        }
        _ => id_event,
    };
    let status = match codes.get(MESSAGE_STATUS).copied() {
        Some("active") => MessageStatus::Active,
        Some("withdrawn") => MessageStatus::Withdrawn,
        other => {
            return Err(invalid_in(
                file,
                MESSAGE_STATUS,
                format!("status {other:?}"),
            ));
        }
    };
    let structure = match structure_element {
        None => {
            defect_in(file, hits, id, Defect::MessageWithoutStructure)?;
            None
        }
        Some(element) => Some(message_structure(file, element, structures, hits)?),
    };
    Ok(Message {
        id: id.to_owned(),
        url: definition.url.clone(),
        code: code.to_owned(),
        event: event.to_owned(),
        structure,
        status,
    })
}

/// The fixed codes of a message definition by element id, and its
/// `Message.structure` element, each checked for its shape and given once.
type MessageElements<'a> = (BTreeMap<&'a str, &'a str>, Option<&'a MessageElement>);

fn message_elements<'a>(
    file: &str,
    elements: &'a [MessageElement],
) -> Result<MessageElements<'a>, LowerError> {
    let mut codes: BTreeMap<&str, &str> = BTreeMap::new();
    let mut structure_element = None;
    for element in elements {
        if element.path != element.id {
            return Err(invalid_in(
                file,
                &element.id,
                format!("path {} differs from the id", element.path),
            ));
        }
        let element_id = element.id.as_str();
        let is_code = [MESSAGE_TYPE, TRIGGER_EVENT, MESSAGE_STATUS].contains(&element_id);
        let is_reference =
            element_id == MESSAGE_STRUCTURE || ACKNOWLEDGEMENTS.contains(&element_id);
        if !is_code && !is_reference {
            return Err(invalid_in(
                file,
                element_id,
                "an element the lowering does not know",
            ));
        }
        let shape_ok = if is_code {
            element.pattern_code.is_some() && element.types.is_none()
        } else {
            element.pattern_code.is_none() && element.types.is_some()
        };
        if !shape_ok {
            return Err(invalid_in(
                file,
                element_id,
                "a code element without a patternCode or a reference element without a type",
            ));
        }
        let duplicate = if let Some(code) = &element.pattern_code {
            codes.insert(element_id, code.as_str()).is_some()
        } else if element_id == MESSAGE_STRUCTURE {
            structure_element.replace(element).is_some()
        } else {
            false
        };
        if duplicate {
            return Err(invalid_in(file, element_id, "the id is given twice"));
        }
    }
    Ok((codes, structure_element))
}

/// The id of the structure `Message.structure` names: the one structure whose
/// id, with every `_` written `-`, is the last step of the target profile.
fn message_structure(
    file: &str,
    element: &MessageElement,
    structures: &BTreeMap<String, Structure>,
    hits: &RefCell<BTreeSet<(Defect, String)>>,
) -> Result<String, LowerError> {
    let profile = match element.types.as_deref() {
        Some([only]) if only.code == "Reference" => match only.target_profile.as_slice() {
            [profile] => profile.as_str(),
            _ => {
                return Err(invalid_in(
                    file,
                    &element.id,
                    "not exactly one target profile",
                ));
            }
        },
        _ => {
            return Err(invalid_in(
                file,
                &element.id,
                "not exactly one Reference type",
            ));
        }
    };
    let Some(name) = profile.strip_prefix(STRUCTURE_PROFILE) else {
        return Err(invalid_in(
            file,
            &element.id,
            format!("target profile {profile} names no message structure"),
        ));
    };
    defect_in(file, hits, &element.id, Defect::StructureProfileName)?;
    let mut matches = structures
        .keys()
        .filter(|candidate| candidate.replace('_', "-") == name);
    match (matches.next(), matches.next()) {
        (Some(only), None) => Ok(only.clone()),
        (None, _) => Err(invalid_in(
            file,
            &element.id,
            format!("target profile {profile} matches no message structure by name"),
        )),
        (Some(first), Some(second)) => Err(invalid_in(
            file,
            &element.id,
            format!("target profile {profile} matches both {first} and {second}"),
        )),
    }
}
