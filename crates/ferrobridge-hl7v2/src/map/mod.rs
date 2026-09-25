// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter of the HL7 v2-to-FHIR `ConceptMaps`.
//!
//! The corpus is the `hl7.fhir.uv.v2mappings` package ([`corpus`]). A parsed
//! message runs through it in the guide's order: the message map of its
//! structure names, per segment, the resource it maps to; the segment map for
//! that segment and resource maps each field; a data type map maps the
//! components of a complex field; and a table map, named by `mappedVia`, is
//! answered by `ConceptMap/$translate` on a terminology server loaded with the
//! guide's table maps, never by a table evaluated here.
//!
//! The FHIR side is written through `fhirconnect::tree::write` over the
//! `fhir-types` R4 element table, so every element, cardinality and choice
//! comes from the generated table. Rows at the same `[n]` fill one instance
//! and rows at different ones fill different instances, allocated in the
//! order a value first reaches them (`mapping_guidelines.md` §\[n\] Notation:
//! the labels are identities, never positions). A `(Type)` reference creates
//! a resource of that type with a `urn:uuid` full url and writes the
//! reference to it.
//!
//! The output is an R4 `message` Bundle: the `MessageHeader` first, then every
//! other resource in the order the run created it
//! (<https://hl7.org/fhir/R4/bundle.html>, `Bundle.type` `message`). Every
//! segment, field and component no row maps, and every condition, target or
//! assignment the interpreter cannot evaluate, is an [`Outcome`] on the
//! result. A value outside its primitive's lexical form is never written, an
//! element or resource lacking a required element is left out, and a
//! resource that still does not decode as R4 is left out too, each counted,
//! so the Bundle decodes. A message whose `MessageHeader` is left out is
//! refused, since a `message` Bundle opens with one (`bdl-12`).

pub mod condition;
mod constraint;
pub mod convert;
pub mod corpus;
pub mod notation;
mod run;

use std::collections::BTreeMap;

use fhir_types::codec::{DecodeError, DecodeErrorKind, Value};
use fhirconnect::tree::error::{ParseError, ResolveError, WriteError};

use crate::map::convert::ConvertError;
use crate::map::corpus::Corpus;
use crate::map::notation::NotationError;
use crate::parse::{Location, Parsed, Unplaced};

/// The row an outcome is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowRef {
    /// The `ConceptMap` id.
    pub map: String,
    /// The source code as written.
    pub source: String,
    /// The target code as written.
    pub target: String,
}

/// Something the run did not carry, typed and counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A segment or field the parse did not place.
    Parse(Unplaced),
    /// A segment no row of the message map names.
    UnmappedSegment {
        /// Where.
        at: Location,
    },
    /// A valued field no row of the segment maps applied to its segment
    /// names.
    UnmappedField {
        /// Where.
        at: Location,
    },
    /// A valued component no row of the data type map names.
    UnmappedComponent {
        /// Where.
        at: Location,
        /// The data type map.
        map: String,
        /// The component position, from 1.
        component: usize,
    },
    /// A row gated by a `Narrative-Condition`.
    NarrativeCondition {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// A row gated by a condition in a form the interpreter does not
    /// evaluate.
    UnsupportedCondition {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The condition as written.
        text: String,
    },
    /// A row whose condition names an operand the scope cannot read.
    UnevaluableCondition {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The operand.
        operand: String,
    },
    /// A row whose target is not in the notation.
    UnsupportedTarget {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// Why.
        error: NotationError,
    },
    /// A row whose target continues after a `(Type)` reference, or whose
    /// source names another segment than the one mapped.
    UnsupportedShape {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// A row whose `assignment` is no literal.
    UnsupportedAssignment {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The assignment as written.
        text: String,
    },
    /// No single segment map maps the segment to the resource the message map
    /// names.
    NoSegmentMap {
        /// Where.
        at: Location,
        /// The resource type.
        resource: String,
        /// The qualified candidates, when there are several.
        candidates: Vec<String>,
    },
    /// No single data type map maps the source type to the target type.
    NoDatatypeMap {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The v2 data type.
        source_type: String,
        /// The FHIR type.
        target_type: String,
    },
    /// A `mappedVia` that names no loaded table map.
    UnresolvedTable {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The `mappedVia` as written.
        mapped_via: String,
    },
    /// A table map was reached and no terminology server is configured.
    NoTerminology {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// The terminology server found no equivalent translation.
    Untranslated {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The table map's canonical url.
        concept_map: String,
    },
    /// A value the target's FHIR type cannot hold.
    Unconvertible {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// Why.
        error: ConvertError,
    },
    /// A further repetition of a field whose target does not repeat.
    RepetitionDropped {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// A data type map row whose `[n].` prefix names another instance of an
    /// element that does not repeat, or of the resource the data type fills.
    UnplacedInstance {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// A complex value read as a primitive, whose later components hold
    /// values the primitive does not carry.
    ComponentsDropped {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// A target path the element table does not resolve.
    UnknownElement {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// Why.
        error: ElementError,
    },
    /// A value the writer refused.
    Unwritable {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// Why.
        error: WriteError,
    },
    /// A value for an element an earlier row already wrote, which keeps the
    /// earlier value; a choice element holding one alternative counts as
    /// written for every other.
    Superseded {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
    },
    /// A `MessageHeader` endpoint written from the facility field because
    /// neither the application nor the network address field is valued:
    /// MSH-4 for `source`, MSH-6 for `destination`. The value replaces the
    /// data-absent-reason extension the guide's rows write on that endpoint.
    FacilityEndpoint {
        /// Where: the facility field.
        at: Location,
        /// The facility field's row of the segment map.
        row: RowRef,
        /// The element path from the table, for example
        /// `MessageHeader.source.endpoint`.
        element: String,
    },
    /// A value outside the lexical form of the FHIR primitive its element
    /// holds (<https://hl7.org/fhir/R4/datatypes.html#primitive>), which is
    /// never written.
    InvalidValue {
        /// Where.
        at: Location,
        /// The row.
        row: RowRef,
        /// The element path from the table, for example
        /// `MessageHeader.source.endpoint`.
        element: String,
        /// The FHIR primitive type, for example `url`.
        fhir_type: String,
        /// The decoder's refusal.
        error: DecodeErrorKind,
    },
    /// An element dropped from a resource, or a resource dropped from the
    /// Bundle, because it lacks an element its definition requires
    /// (<https://hl7.org/fhir/R4/elementdefinition.html>,
    /// `ElementDefinition.min`).
    MissingRequired {
        /// The resource type.
        resource: String,
        /// The entry's full url.
        full_url: String,
        /// The instance path of what was dropped, the resource type when the
        /// resource itself was.
        element: String,
        /// The definition path of the required element it lacks.
        required: String,
    },
    /// A resource that does not decode as R4, which is left out of the
    /// Bundle.
    Undecodable {
        /// The resource type.
        resource: String,
        /// The entry's full url.
        full_url: String,
        /// Why.
        error: DecodeError,
    },
}

impl Outcome {
    /// Returns a short name of the outcome's kind, for a count.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Parse(unplaced) => unplaced.kind(),
            Self::UnmappedSegment { .. } => "unmapped-segment",
            Self::UnmappedField { .. } => "unmapped-field",
            Self::UnmappedComponent { .. } => "unmapped-component",
            Self::NarrativeCondition { .. } => "narrative-condition",
            Self::UnsupportedCondition { .. } => "unsupported-condition",
            Self::UnevaluableCondition { .. } => "unevaluable-condition",
            Self::UnsupportedTarget { .. } => "unsupported-target",
            Self::UnsupportedShape { .. } => "unsupported-shape",
            Self::UnsupportedAssignment { .. } => "unsupported-assignment",
            Self::NoSegmentMap { .. } => "no-segment-map",
            Self::NoDatatypeMap { .. } => "no-datatype-map",
            Self::UnresolvedTable { .. } => "unresolved-table",
            Self::NoTerminology { .. } => "no-terminology",
            Self::Untranslated { .. } => "untranslated",
            Self::Unconvertible { .. } => "unconvertible",
            Self::RepetitionDropped { .. } => "repetition-dropped",
            Self::UnplacedInstance { .. } => "unplaced-instance",
            Self::ComponentsDropped { .. } => "components-dropped",
            Self::UnknownElement { .. } => "unknown-element",
            Self::Unwritable { .. } => "unwritable",
            Self::Superseded { .. } => "superseded",
            Self::FacilityEndpoint { .. } => "facility-endpoint",
            Self::InvalidValue { .. } => "invalid-value",
            Self::MissingRequired { .. } => "missing-required",
            Self::Undecodable { .. } => "undecodable",
        }
    }
}

/// A refusal to map a message.
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    /// The corpus carries no message map for the message's structure.
    #[error("no message map names segments of the structure {structure}")]
    NoMessageMap {
        /// The structure name, for example `ORU_R01`.
        structure: String,
    },
    /// A `NOT VALUED ERROR` condition held, which stops the mapper.
    #[error("the condition of {} in {} stops the mapper: {operand} is not valued", row.source, row.map)]
    Stopped {
        /// Where.
        at: Box<Location>,
        /// The row.
        row: Box<RowRef>,
        /// The operand.
        operand: String,
    },
    /// The terminology server refused a translation or could not be reached.
    #[error("the translation through {concept_map} failed")]
    Terminology {
        /// The table map's canonical url.
        concept_map: String,
        /// The cause, with the upstream status and body.
        #[source]
        source: Box<ferrobridge_term::error::Error>,
    },
    /// The run completed no `MessageHeader`, which a `message` Bundle holds
    /// as its first resource (<https://hl7.org/fhir/R4/bundle.html#invs>,
    /// `bdl-12`).
    #[error(
        "the message Bundle would carry no MessageHeader (FHIR R4 bdl-12){}",
        lacking(dropped.as_deref())
    )]
    NoMessageHeader {
        /// The outcome that left the `MessageHeader` out, `None` when the run
        /// created none.
        dropped: Option<Box<Outcome>>,
    },
}

/// The required element whose absence left the `MessageHeader` out, as a
/// clause of [`MapError::NoMessageHeader`]'s message.
fn lacking(dropped: Option<&Outcome>) -> String {
    match dropped {
        Some(Outcome::MissingRequired { required, .. }) => format!(": it lacks {required}"),
        Some(Outcome::Undecodable { .. }) => String::from(": it does not decode as R4"),
        _ => String::new(),
    }
}

/// Why a target path names no element.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ElementError {
    /// The path does not parse as a FHIR path.
    #[error("the target path does not parse")]
    Parse(#[from] ParseError),
    /// The element table does not resolve the path.
    #[error("the element table does not resolve the target path")]
    Resolve(#[from] ResolveError),
}

/// A mapped message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapped {
    bundle: Value,
    outcomes: Vec<Outcome>,
    translations: usize,
}

impl Mapped {
    /// Returns the R4 message Bundle.
    #[must_use]
    pub const fn bundle(&self) -> &Value {
        &self.bundle
    }

    /// Returns the Bundle, consuming the result.
    #[must_use]
    pub fn into_bundle(self) -> Value {
        self.bundle
    }

    /// Returns every outcome, in the order the run met them.
    #[must_use]
    pub fn outcomes(&self) -> &[Outcome] {
        &self.outcomes
    }

    /// Returns the number of outcomes of each kind.
    #[must_use]
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for outcome in &self.outcomes {
            let count = counts.entry(outcome.kind()).or_insert(0usize);
            *count = count.saturating_add(1);
        }
        counts
    }

    /// Returns how many `ConceptMap/$translate` calls the run made.
    ///
    /// A table value costs one call per target code system its table map
    /// names (`ConceptMap.group.target`), in group order, until one answers
    /// with an equivalent match.
    #[must_use]
    pub const fn translations(&self) -> usize {
        self.translations
    }
}

/// Maps `parsed` through `corpus`, translating table values on
/// `terminology`.
///
/// # Errors
///
/// Returns [`MapError::NoMessageMap`] when the corpus has no message map for
/// the structure, [`MapError::Stopped`] when a `NOT VALUED ERROR` condition
/// holds, [`MapError::Terminology`] when the terminology server refuses a
/// translation or cannot be reached, and [`MapError::NoMessageHeader`] when
/// the message Bundle would carry no `MessageHeader`.
pub async fn map(
    parsed: &Parsed,
    corpus: &Corpus,
    terminology: Option<&ferrobridge_term::client::Client>,
) -> Result<Mapped, MapError> {
    let mut run = run::Run::new(corpus, parsed);
    run.message()?;
    let translations = run.translate(terminology).await?;
    let (bundle, outcomes) = run.finish()?;
    Ok(Mapped {
        bundle,
        outcomes,
        translations,
    })
}
