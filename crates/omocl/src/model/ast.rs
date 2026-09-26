// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL file model.
//!
//! One Rust type per construct an OMOCL v1.0.0 mapping file holds. OMOCL
//! publishes its grammar as two railroad images and four syntax tables
//! (<https://github.com/SevKohler/OMOCL/wiki/Syntax-and-grammar>, vendored under
//! `docs/specs/omocl/docs/wiki-images/`) and no schema, so the shapes here are
//! read from those artefacts first and from the vendored mapping library
//! second. Every node keeps the [`Position`] of the YAML it was read from.
//!
//! The nodes are plain records: [`crate::model::parse`] builds them and nothing
//! else mutates them, so their fields are public and every invariant the
//! grammar fixes is carried by the field's own type.

use core::fmt;
use core::str::FromStr;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use openehr_mapping_core::header::Header;
use openehr_mapping_core::header::archetype::ArchetypeId;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;

use crate::model::projection::KeyProjection;

/// A CDM table an OMOCL record writes into.
///
/// The spellings are the values the library writes in a record's `type`; the
/// railroad calls the key `type` and the syntax tables head each table with the
/// same spelling (`DrugExposure`, `ConditionOccurrence`, `ProcedureOccurrence`,
/// `Death`). `Visit` is the library's spelling for the CDM's
/// `VISIT_OCCURRENCE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Target {
    /// `MEASUREMENT`.
    Measurement,
    /// `OBSERVATION`.
    Observation,
    /// `DRUG_EXPOSURE`.
    DrugExposure,
    /// `CONDITION_OCCURRENCE`.
    ConditionOccurrence,
    /// `PROCEDURE_OCCURRENCE`.
    ProcedureOccurrence,
    /// `DEVICE_EXPOSURE`.
    DeviceExposure,
    /// `SPECIMEN`.
    Specimen,
    /// `VISIT_OCCURRENCE`.
    Visit,
    /// `DEATH`.
    Death,
    /// `PERSON`.
    Person,
}

impl Target {
    /// Every target, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Measurement,
        Self::Observation,
        Self::DrugExposure,
        Self::ConditionOccurrence,
        Self::ProcedureOccurrence,
        Self::DeviceExposure,
        Self::Specimen,
        Self::Visit,
        Self::Death,
        Self::Person,
    ];

    /// Returns the spelling a record's `type` writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Measurement => "Measurement",
            Self::Observation => "Observation",
            Self::DrugExposure => "DrugExposure",
            Self::ConditionOccurrence => "ConditionOccurrence",
            Self::ProcedureOccurrence => "ProcedureOccurrence",
            Self::DeviceExposure => "DeviceExposure",
            Self::Specimen => "Specimen",
            Self::Visit => "Visit",
            Self::Death => "Death",
            Self::Person => "Person",
        }
    }

    /// Returns the CDM table name, as `omop_cdm::meta::table` spells it.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::Measurement => "measurement",
            Self::Observation => "observation",
            Self::DrugExposure => "drug_exposure",
            Self::ConditionOccurrence => "condition_occurrence",
            Self::ProcedureOccurrence => "procedure_occurrence",
            Self::DeviceExposure => "device_exposure",
            Self::Specimen => "specimen",
            Self::Visit => "visit_occurrence",
            Self::Death => "death",
            Self::Person => "person",
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The value a mapping entry's `type` writes: one of twelve.
///
/// Ten name a CDM table and two are structural: `Include` (the railroad's
/// INCLUDE clause) and `CustomMapping`, which no grammar artefact names and one
/// library file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EntityType {
    /// A record writing one row into a CDM table.
    Target(Target),
    /// `Include`: the mappings of another archetype, applied below a path.
    Include,
    /// `CustomMapping`: a first-party converter named by the file.
    CustomMapping,
}

impl EntityType {
    /// Every spelling a `type` admits, in declaration order.
    pub const ADMITTED: &'static [&'static str] = &[
        "Measurement",
        "Observation",
        "DrugExposure",
        "ConditionOccurrence",
        "ProcedureOccurrence",
        "DeviceExposure",
        "Specimen",
        "Visit",
        "Death",
        "Person",
        "Include",
        "CustomMapping",
    ];

    /// Returns the spelling a `type` writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Target(target) => target.as_str(),
            Self::Include => "Include",
            Self::CustomMapping => "CustomMapping",
        }
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a `type` value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is none of the twelve entity types")]
pub struct EntityTypeError {
    /// The refused value, as the file writes it.
    pub value: String,
}

impl FromStr for EntityType {
    type Err = EntityTypeError;

    // NOTE: no specification governs this: our own design; a `type` compares
    // case-exactly, because every library file writes the one spelling.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(target) = Target::ALL
            .iter()
            .copied()
            .find(|target| target.as_str() == value)
        {
            return Ok(Self::Target(target));
        }
        match value {
            "Include" => Ok(Self::Include),
            "CustomMapping" => Ok(Self::CustomMapping),
            _ => Err(EntityTypeError {
                value: value.to_owned(),
            }),
        }
    }
}

/// An OMOP `concept_id`, as a literal `code` or a `conceptMap` value writes it.
///
/// The CDM types every `*_concept_id` column `integer`
/// (`docs/specs/omop-cdm/inst/csv/OMOP_CDMv5.4_Field_Level.csv`), so a concept
/// id is a 32-bit signed integer. `0` is the concept the CDM's own guidance
/// writes when a source code has no standard concept ("a `DRUG_CONCEPT_ID` of
/// 0", `drug_exposure.drug_concept_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConceptId(i32);

impl ConceptId {
    /// The concept the CDM writes when no standard concept matches.
    pub const NO_MATCHING_CONCEPT: Self = Self(0);

    /// Creates a concept id.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Returns the number.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl fmt::Display for ConceptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An OMOP vocabulary domain, as `CONCEPT.domain_id` spells it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainId(String);

impl DomainId {
    /// Creates a domain id from its spelling.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DomainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An archetype at-code, the key a `conceptMap` `mapping` writes.
///
/// The form is `at` followed by four digits and any number of `.n`
/// specialisation parts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AtCode(String);

/// Why an at-code was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not `at` followed by four digits and optional `.n` parts")]
pub struct AtCodeError {
    /// The refused value.
    pub value: String,
}

impl AtCode {
    /// Returns the at-code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AtCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AtCode {
    type Err = AtCodeError;

    // NOTE: no specification governs this: our own design; OMOCL is silent on
    // the key form and every library key is `at` and four digits.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let refuse = || AtCodeError {
            value: value.to_owned(),
        };
        let digits = value.strip_prefix("at").ok_or_else(refuse)?;
        let mut parts = digits.split('.');
        let head = parts.next().ok_or_else(refuse)?;
        if head.len() != 4 || !head.bytes().all(|b| b.is_ascii_digit()) {
            return Err(refuse());
        }
        if !parts.all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())) {
            return Err(refuse());
        }
        Ok(Self(value.to_owned()))
    }
}

/// The CDM a file's `spec.system` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CdmSystem {
    /// `OMOP`, the one system every library file names.
    Omop,
}

/// The CDM version a file's `spec.version` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CdmVersion {
    /// `5.4`, the one version every library file names and the CDM release
    /// this crate's `omop-cdm` dependency is generated from.
    V5_4,
}

/// The `spec` block below the shared header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    /// Where the block starts.
    pub position: Position,
    /// `spec.system`.
    pub system: Located<CdmSystem>,
    /// `spec.version`.
    pub version: Located<CdmVersion>,
}

/// One OMOCL mapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingFile {
    /// The file the mapping was loaded from.
    pub file: PathBuf,
    /// The shared header.
    pub header: Header,
    /// The mapped archetype, `spec.openEhrConfig.archetype`.
    pub archetype: Located<ArchetypeId>,
    /// The OMOCL half of `spec`.
    pub spec: Spec,
    /// The entries of `mappings`, in file order.
    pub entities: Vec<Entity>,
}

impl MappingFile {
    /// Returns the file the mapping was loaded from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Returns the shared header.
    #[must_use]
    pub const fn header(&self) -> &Header {
        &self.header
    }
}

/// One entry of `mappings`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Entity {
    /// A record writing into a CDM table.
    Record(Record),
    /// An `Include`.
    Include(Include),
    /// A `CustomMapping`.
    CustomMapping(CustomMapping),
}

impl Entity {
    /// Returns where the entry starts.
    #[must_use]
    pub const fn position(&self) -> Position {
        match *self {
            Self::Record(ref record) => record.position,
            Self::Include(ref include) => include.position,
            Self::CustomMapping(ref custom) => custom.position,
        }
    }

    /// Returns the entry's `type`.
    #[must_use]
    pub const fn entity_type(&self) -> EntityType {
        match *self {
            Self::Record(ref record) => EntityType::Target(*record.target.value()),
            Self::Include(_) => EntityType::Include,
            Self::CustomMapping(_) => EntityType::CustomMapping,
        }
    }
}

/// A record: the railroad's ENTITY clause under a CDM target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Where the entry starts.
    pub position: Position,
    /// The `type`.
    pub target: Located<Target>,
    /// `base_path`, the railroad's ITERATION clause.
    pub base_path: Option<Located<MappingPath>>,
    /// The column entries, in key order.
    pub columns: Vec<Column>,
}

/// One column entry of a record: a key and its CONVERSION clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Where the key is written.
    pub key_position: Position,
    /// The projection the key names, which carries the key's spelling.
    pub projection: &'static KeyProjection,
    /// `optional`, when the entry writes it.
    pub optional: Option<Located<bool>>,
    /// `alternatives`, in file order.
    pub alternatives: Vec<Alternative>,
}

impl Column {
    /// Returns the key as the file writes it.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        self.projection.key
    }

    /// Whether the entry writes `optional: true`.
    #[must_use]
    pub fn is_optional(&self) -> bool {
        self.optional.is_some_and(|optional| *optional.value())
    }
}

/// One alternative of an `alternatives` list: exactly one of four forms.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Alternative {
    /// `path`: a value read from the composition.
    Path(Located<MappingPath>),
    /// `code`: a literal concept id.
    Code(Located<ConceptId>),
    /// `conceptMap`: an at-code to concept id table over one path.
    ConceptMap(ConceptMap),
    /// `multiplication`: the product of the values its paths read.
    Multiplication(Multiplication),
}

/// A `conceptMap` alternative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptMap {
    /// Where the `conceptMap` key is written.
    pub position: Position,
    /// `path`, the coded node the at-code is read from.
    pub path: Located<MappingPath>,
    /// `mapping`, the at-code to concept id table.
    pub mapping: BTreeMap<AtCode, Located<ConceptId>>,
}

/// A `multiplication` alternative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multiplication {
    /// Where the `multiplication` key is written.
    pub position: Position,
    /// The factors, in file order.
    pub factors: Vec<Factor>,
}

/// One factor of a `multiplication`: exactly one of `path` and `code`.
///
/// The OMOCL railroad (`docs/specs/omocl/docs/wiki-images/omop_railroad.png`,
/// CONVERSION clause) follows `multiplication` with CDM FIELD clauses, of which
/// `path` and `code` are the two that name a value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Factor {
    /// `path`: a number read from the composition.
    Path(Located<MappingPath>),
    /// `code`: a literal integer factor.
    Code(Located<i64>),
}

/// An `Include` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Include {
    /// Where the entry starts.
    pub position: Position,
    /// `archetype_id`, the archetype whose mappings apply.
    pub archetype_id: Located<ArchetypeId>,
    /// `base_path`, where in this archetype the included one sits.
    pub base_path: Option<Located<MappingPath>>,
}

/// A `CustomMapping` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomMapping {
    /// Where the entry starts.
    pub position: Position,
    /// `name`, the converter the entry runs.
    pub name: Located<String>,
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::AtCode;
    use super::EntityType;
    use super::Target;

    #[test]
    fn every_admitted_spelling_parses_and_renders_back() {
        for spelling in EntityType::ADMITTED {
            let parsed = EntityType::from_str(spelling).expect("an admitted spelling");
            assert_eq!(parsed.as_str(), *spelling);
        }
        assert_eq!(EntityType::ADMITTED.len(), 12);
    }

    #[test]
    fn a_type_compares_case_exactly() {
        assert!(EntityType::from_str("measurement").is_err());
        assert!(EntityType::from_str("VisitOccurrence").is_err());
    }

    #[test]
    fn a_target_names_its_cdm_table() {
        assert_eq!(Target::Visit.table(), "visit_occurrence");
        assert_eq!(Target::ALL.len(), 10);
    }

    #[test]
    fn an_at_code_is_at_and_four_digits() {
        assert!(AtCode::from_str("at0003").is_ok());
        assert!(AtCode::from_str("at0003.1").is_ok());
        for refused in ["at003", "ac0003", "at0003.", "at00031", "0003", "at0003.x"] {
            assert!(AtCode::from_str(refused).is_err(), "{refused}");
        }
    }
}
