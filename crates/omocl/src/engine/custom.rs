// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first-party converters a `CustomMapping` names.
//!
//! No OMOCL grammar artefact names `CustomMapping`; the library uses it once,
//! for `FactRelationshipCustomConverter`
//! (`docs/specs/omocl/medical_data/observation/Laboratory_test_result_v1.yml`).
//! A converter sees the rows one instance of its scope produced, the rows of
//! every scope it includes among them, and answers the pairs of rows a
//! `FACT_RELATIONSHIP` links; the engine fills the domain concepts, and the
//! writer writes each pair in both directions.

use core::fmt;
use std::sync::Arc;

use omop_cdm::graph::key::RecordKey;
use omop_cdm::graph::row::Row;

use crate::model::semantic::ConverterRegistry;
use crate::model::semantic::FirstPartyConverters;

/// One row of a relationship: its CDM table and its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// The CDM table of the row.
    pub table: &'static str,
    /// The row's key.
    pub key: RecordKey,
}

impl Fact {
    /// Returns the fact of `row`.
    #[must_use]
    pub fn of(row: &Row) -> Self {
        Self {
            table: row.table().name,
            key: row.key().clone(),
        }
    }
}

/// One relationship between two rows a converter answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// The row `fact_id_1` names in the first direction.
    pub first: Fact,
    /// The row `fact_id_2` names in the first direction.
    pub second: Fact,
}

/// A first-party converter over the rows of one scope instance.
pub trait CustomConverter: fmt::Debug + Send + Sync {
    /// Returns the name a `CustomMapping` writes for the converter.
    fn name(&self) -> &'static str;

    /// Returns the relationships among `rows`, the rows one instance of the
    /// converter's scope produced, in production order.
    fn relate(&self, rows: &[&Row]) -> Vec<Relation>;
}

/// A registry that answers a converter object for a name.
pub trait Converters: ConverterRegistry {
    /// Returns the converter registered under `name`.
    fn converter(&self, name: &str) -> Option<Arc<dyn CustomConverter>>;
}

impl Converters for FirstPartyConverters {
    fn converter(&self, name: &str) -> Option<Arc<dyn CustomConverter>> {
        match name {
            FactRelationshipCustomConverter::NAME => {
                Some(Arc::new(FactRelationshipCustomConverter))
            }
            _ => None,
        }
    }
}

/// `FactRelationshipCustomConverter`: relates every `MEASUREMENT` row of a
/// laboratory result to its `SPECIMEN` row.
///
/// The CDM names "measurements derived from an associated specimen" as a
/// fact relationship (<https://ohdsi.github.io/CommonDataModel/cdm54.html#FACT_RELATIONSHIP>).
/// The laboratory result and a panel write no row of their own, so the
/// specimen of the same result instance is the row an analyte relates to.
/// No concept names the relationship, so it is written with
/// `relationship_concept_id = 0`, the gap Kohler et al. record
/// (arXiv:2607.27208).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FactRelationshipCustomConverter;

impl FactRelationshipCustomConverter {
    /// The name the library writes.
    pub const NAME: &'static str = "FactRelationshipCustomConverter";
}

impl CustomConverter for FactRelationshipCustomConverter {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn relate(&self, rows: &[&Row]) -> Vec<Relation> {
        // NOTE: CDM FACT_RELATIONSHIP, "facts derived from one another (measurements
        // derived from an associated specimen)"; the writer adds the reverse row.
        let of = |table: &str| -> Vec<Fact> {
            rows.iter()
                .filter(|row| row.table().name == table)
                .map(|row| Fact::of(row))
                .collect()
        };
        let specimens = of("specimen");
        let mut relations = Vec::new();
        for measurement in of("measurement") {
            for specimen in &specimens {
                relations.push(Relation {
                    first: measurement.clone(),
                    second: specimen.clone(),
                });
            }
        }
        relations
    }
}
