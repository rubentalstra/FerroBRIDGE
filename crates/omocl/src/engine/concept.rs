// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The vocabulary the engine resolves concepts through, behind one trait.
//!
//! The CDM sink wires the SQL resolver of `omop-cdm` in; the engine tests
//! wire a map. The question and the answer are `omop_cdm::vocabulary`'s own
//! types, which need no database. The engine asks once per distinct question
//! within a run and keeps no answer across runs.

use core::fmt;
use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use omop_cdm::value::CdmDate;
use omop_cdm::vocabulary::Resolution;
use omop_cdm::vocabulary::ResolveError;
use omop_cdm::vocabulary::SourceKey;

use crate::model::ast::ConceptId;

/// A comparison a `DV_QUANTITY`'s `magnitude_status` writes that the CDM has
/// an operator concept for.
///
/// The CDM names the operators `<`, `<=`, `=`, `>=` and `>` in the
/// `Meas Value Operator` domain and asks for `operator_concept_id` to be left
/// NULL for an exact value (`OMOP_CDMv5.4_Field_Level.csv`,
/// `measurement.operator_concept_id`), so `=` has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Operator {
    /// `<`.
    Less,
    /// `<=`.
    LessOrEqual,
    /// `>=`.
    GreaterOrEqual,
    /// `>`.
    Greater,
}

impl Operator {
    /// Returns the operator as `magnitude_status` and the CDM write it.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::GreaterOrEqual => ">=",
            Self::Greater => ">",
        }
    }

    /// Reads a `magnitude_status`, `None` for one the CDM has no concept for.
    #[must_use]
    pub fn from_status(status: &str) -> Option<Self> {
        match status {
            "<" => Some(Self::Less),
            "<=" => Some(Self::LessOrEqual),
            ">=" => Some(Self::GreaterOrEqual),
            ">" => Some(Self::Greater),
            _ => None,
        }
    }
}

impl fmt::Display for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// A vocabulary lookup that failed upstream.
#[derive(Debug, thiserror::Error)]
#[error("the vocabulary lookup of {what} failed")]
pub struct LookupError {
    /// What was looked up.
    pub what: String,
    /// The upstream failure.
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

/// The vocabulary the engine resolves concepts through.
pub trait ConceptSource: Sync {
    /// Resolves a source code on the record date, as
    /// `omop_cdm::vocabulary::ConceptResolver::resolve` resolves it.
    fn resolve(
        &self,
        key: &SourceKey,
        date: &CdmDate,
    ) -> impl Future<Output = Result<Resolution, ResolveError>> + Send;

    /// Returns the standard concept of an operator in the `Meas Value
    /// Operator` domain valid on the record date, `None` when the vocabulary
    /// holds none.
    fn operator(
        &self,
        operator: Operator,
        date: &CdmDate,
    ) -> impl Future<Output = Result<Option<ConceptId>, LookupError>> + Send;

    /// Returns the `domain_concept_id` of a domain, as the `DOMAIN` table
    /// holds it, `None` when it holds no such domain.
    fn domain_concept(
        &self,
        domain: &str,
    ) -> impl Future<Output = Result<Option<ConceptId>, LookupError>> + Send;
}

/// The OHDSI vocabulary a `DV_QUANTITY.units` string is looked up in.
pub const UNIT_VOCABULARY: &str = "UCUM";

/// The reading of an openEHR terminology id as an OHDSI `vocabulary_id`.
///
/// No specification governs this: our own design. An openEHR
/// `TERMINOLOGY_ID` and an OHDSI `vocabulary_id` name the same code system
/// differently (`SNOMED-CT` against `SNOMED`), so the engine reads the id
/// through this table, and an id it does not list is taken as the
/// `vocabulary_id` itself. The defaults are `LOINC` to `LOINC` and
/// `SNOMED-CT` to `SNOMED`; a deployment adds its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyAliases(BTreeMap<String, String>);

impl VocabularyAliases {
    /// Creates a table with no alias.
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeMap::new())
    }

    /// Returns this table with `terminology` read as `vocabulary`.
    #[must_use]
    pub fn with(mut self, terminology: impl Into<String>, vocabulary: impl Into<String>) -> Self {
        self.0.insert(terminology.into(), vocabulary.into());
        self
    }

    /// Returns the `vocabulary_id` an openEHR terminology id is read as.
    #[must_use]
    pub fn vocabulary_of<'a>(&'a self, terminology: &'a str) -> &'a str {
        self.0.get(terminology).map_or(terminology, String::as_str)
    }
}

impl Default for VocabularyAliases {
    fn default() -> Self {
        Self::empty()
            .with("LOINC", "LOINC")
            .with("SNOMED-CT", "SNOMED")
    }
}
