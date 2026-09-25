// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The run report: what one ETL run wrote, counted per mapping and in total,
//! and every record it refused with the reason.
//!
//! The counts are the ones the architecture fixes for a run: rows per table,
//! `concept_id = 0` assignments, `relationship_concept_id = 0` links, source
//! fields no mapping wrote, and refusals. The report renders as text for a
//! person and as JSON for a machine. No specification governs its shape: our
//! own design.

use crate::etl::tie::Tie;
use omop_cdm::graph::{Refusal, Report, Source};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

/// The counts of one mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MappingTotals {
    /// Rows written, by table.
    pub rows: BTreeMap<String, u64>,
    /// `concept_id = 0` assignments, by `table.column`.
    pub concept_zero: BTreeMap<String, u64>,
    /// Source fields the mapping wrote nowhere.
    pub unmapped_fields: u64,
    /// Records refused.
    pub refusals: u64,
}

/// One refused record, as the report lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusalEntry {
    /// The versioned composition, when the refusal belongs to one.
    pub composition: Option<String>,
    /// The version, when the refusal belongs to one.
    pub version: Option<String>,
    /// The mapping.
    pub mapping: Option<String>,
    /// The CDM table.
    pub table: Option<String>,
    /// The CDM column.
    pub column: Option<String>,
    /// The openEHR element, as an RM path.
    pub element: Option<String>,
    /// Why.
    pub reason: String,
}

/// One source field no mapping wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnmappedEntry {
    /// The versioned composition.
    pub composition: String,
    /// The mapping.
    pub mapping: String,
    /// The element, as an RM path.
    pub element: String,
}

/// The compositions a run read, and what became of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Compositions {
    /// Rows the composition query answered.
    pub read: u64,
    /// Compositions committed.
    pub committed: u64,
    /// Compositions skipped because the watermark names the same version.
    pub skipped: u64,
    /// Compositions refused whole.
    pub refused: u64,
    /// Committed compositions tied to a derived visit.
    pub with_visit: u64,
    /// Committed compositions inside no derived visit of their EHR.
    pub without_visit: u64,
    /// Committed compositions inside several derived visits that the
    /// facility did not decide between, written with no visit.
    pub ambiguous_visit: u64,
}

/// The rows each derived table holds after the run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Derived {
    /// `VISIT_OCCURRENCE` rows the visit query wrote.
    pub visit_occurrence: u64,
    /// `OBSERVATION_PERIOD` rows.
    pub observation_period: u64,
    /// `CONDITION_ERA` rows.
    pub condition_era: u64,
    /// `DRUG_ERA` rows.
    pub drug_era: u64,
}

/// What one ETL run did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunReport {
    /// The run's identifier, recorded with every watermark it wrote.
    pub run_id: String,
    /// The compositions read.
    pub compositions: Compositions,
    /// Rows written, by table, over every mapping.
    pub rows: BTreeMap<String, u64>,
    /// `concept_id = 0` assignments over every mapping.
    pub concept_zero: u64,
    /// `FACT_RELATIONSHIP` rows written with `relationship_concept_id = 0`.
    pub zero_relationship_links: u64,
    /// The counts of each mapping.
    pub mappings: BTreeMap<String, MappingTotals>,
    /// The derived tables.
    pub derived: Derived,
    /// Every refused record.
    pub refusals: Vec<RefusalEntry>,
    /// Every source field no mapping wrote.
    pub unmapped_fields: Vec<UnmappedEntry>,
}

impl RunReport {
    /// Starts the report of the run `run_id`.
    #[must_use]
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            ..Self::default()
        }
    }

    /// Adds the report of one committed composition.
    pub fn committed(&mut self, source: &Source, report: &Report) {
        self.compositions.committed = self.compositions.committed.saturating_add(1);
        for ((mapping, table), rows) in report.rows() {
            add(self.rows.entry((*table).to_owned()).or_default(), *rows);
            let totals = self.mappings.entry(mapping.to_string()).or_default();
            add(totals.rows.entry((*table).to_owned()).or_default(), *rows);
        }
        for ((mapping, table, column), count) in report.concept_zero() {
            self.concept_zero = self.concept_zero.saturating_add(*count);
            let totals = self.mappings.entry(mapping.to_string()).or_default();
            add(
                totals
                    .concept_zero
                    .entry(format!("{table}.{column}"))
                    .or_default(),
                *count,
            );
        }
        self.zero_relationship_links = self
            .zero_relationship_links
            .saturating_add(report.zero_relationship_links());
        self.engine_outcomes(Some(source), report);
    }

    /// Adds the unmapped fields and the record refusals the engine counted
    /// for one composition, whether or not it committed.
    pub fn engine_outcomes(&mut self, source: Option<&Source>, report: &Report) {
        for field in report.unmapped_fields() {
            let totals = self
                .mappings
                .entry(field.mapping().to_string())
                .or_default();
            totals.unmapped_fields = totals.unmapped_fields.saturating_add(1);
            self.unmapped_fields.push(UnmappedEntry {
                composition: source.map_or_else(String::new, |source| {
                    source.versioned_object_uid().to_string()
                }),
                mapping: field.mapping().to_string(),
                element: field.element().to_owned(),
            });
        }
        for refusal in report.refusals() {
            self.refusal(source, refusal);
        }
    }

    /// Adds one refused record, or one refused composition when the refusal
    /// names no table.
    pub fn refusal(&mut self, source: Option<&Source>, refusal: &Refusal) {
        if let Some(mapping) = refusal.mapping() {
            let totals = self.mappings.entry(mapping.to_string()).or_default();
            totals.refusals = totals.refusals.saturating_add(1);
        }
        self.refusals.push(RefusalEntry {
            composition: source.map(|source| source.versioned_object_uid().to_string()),
            version: source.map(|source| source.version_uid().to_string()),
            mapping: refusal.mapping().map(ToString::to_string),
            table: refusal.table().map(str::to_owned),
            column: refusal.column().map(str::to_owned),
            element: refusal.element().map(str::to_owned),
            reason: refusal.reason().to_owned(),
        });
    }

    /// Counts the visit tie of one committed composition; a run that derives
    /// no visits ties nothing and counts nothing.
    pub fn tied(&mut self, tie: Option<&Tie>) {
        let count = match tie {
            Some(Tie::Visit(_)) => &mut self.compositions.with_visit,
            Some(Tie::Outside) => &mut self.compositions.without_visit,
            Some(Tie::Ambiguous) => &mut self.compositions.ambiguous_visit,
            None => return,
        };
        *count = count.saturating_add(1);
    }

    /// Adds a composition refused whole.
    pub fn composition_refused(&mut self, source: Option<&Source>, refusal: &Refusal) {
        self.compositions.refused = self.compositions.refused.saturating_add(1);
        self.refusal(source, refusal);
    }
}

/// Adds `count` to `total`, saturating.
fn add(total: &mut u64, count: u64) {
    *total = total.saturating_add(count);
}

impl fmt::Display for RunReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let compositions = &self.compositions;
        writeln!(f, "run {}", self.run_id)?;
        writeln!(
            f,
            "compositions: {} read, {} committed, {} skipped, {} refused",
            compositions.read, compositions.committed, compositions.skipped, compositions.refused
        )?;
        writeln!(
            f,
            "visits: {} compositions tied, {} inside no visit, {} inside several",
            compositions.with_visit, compositions.without_visit, compositions.ambiguous_visit
        )?;
        for (table, rows) in &self.rows {
            writeln!(f, "rows {table}: {rows}")?;
        }
        writeln!(f, "concept 0 assignments: {}", self.concept_zero)?;
        writeln!(
            f,
            "fact relationships with relationship_concept_id 0: {}",
            self.zero_relationship_links
        )?;
        let derived = &self.derived;
        writeln!(
            f,
            "derived: visit_occurrence {}, observation_period {}, condition_era {}, drug_era {}",
            derived.visit_occurrence,
            derived.observation_period,
            derived.condition_era,
            derived.drug_era
        )?;
        for (mapping, totals) in &self.mappings {
            let rows = totals
                .rows
                .values()
                .fold(0_u64, |sum, rows| sum.saturating_add(*rows));
            let zero = totals
                .concept_zero
                .values()
                .fold(0_u64, |sum, zero| sum.saturating_add(*zero));
            writeln!(
                f,
                "mapping {mapping}: {rows} rows, {zero} concept 0, {} unmapped fields, {} refusals",
                totals.unmapped_fields, totals.refusals
            )?;
        }
        for refusal in &self.refusals {
            let place = [
                refusal.composition.as_deref(),
                refusal.mapping.as_deref(),
                refusal.table.as_deref(),
                refusal.column.as_deref(),
                refusal.element.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            writeln!(f, "refused {place}: {}", refusal.reason)?;
        }
        Ok(())
    }
}
