// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outcomes the engine counts and the writer completes.

use std::collections::BTreeMap;

use crate::graph::key::MappingName;
use crate::graph::row::GraphError;

/// A record the engine or the writer refused, and why.
///
/// The fields name where the record came from and never carry a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    mapping: Option<MappingName>,
    table: Option<&'static str>,
    column: Option<&'static str>,
    element: Option<String>,
    reason: String,
}

impl Refusal {
    /// Records a refusal for `reason`.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            mapping: None,
            table: None,
            column: None,
            element: None,
            reason: reason.into(),
        }
    }

    /// Records a refusal of a row the graph refused to build.
    #[must_use]
    pub fn of_row(error: &GraphError) -> Self {
        let (table, column) = match error {
            GraphError::NotWritable { table, .. }
            | GraphError::ForeignRow { table, .. }
            | GraphError::DuplicateRow { table, .. }
            | GraphError::DanglingLink { table, .. } => (Some(*table), None),
            GraphError::PrimaryKey { table, column }
            | GraphError::Duplicate { table, column }
            | GraphError::Mismatch { table, column, .. }
            | GraphError::TooLong { table, column, .. }
            | GraphError::NotFinite { table, column }
            | GraphError::Reference { table, column, .. }
            | GraphError::Missing { table, column } => (Some(*table), Some(*column)),
            GraphError::UnknownTable { .. } | GraphError::UnknownColumn { .. } => (None, None),
        };
        Self {
            mapping: None,
            table,
            column,
            element: None,
            reason: error.to_string(),
        }
    }

    /// Names the mapping the record came from.
    #[must_use]
    pub fn with_mapping(mut self, mapping: MappingName) -> Self {
        self.mapping = Some(mapping);
        self
    }

    /// Names the CDM table the refused record would have written.
    #[must_use]
    pub fn with_table(mut self, table: &'static str) -> Self {
        self.table = Some(table);
        self
    }

    /// Names the column the refusal is about: an OMOCL key or a CDM column.
    #[must_use]
    pub fn with_column(mut self, column: &'static str) -> Self {
        self.column = Some(column);
        self
    }

    /// Names the openEHR element, as an RM path, the refusal is about.
    #[must_use]
    pub fn with_element(mut self, element: impl Into<String>) -> Self {
        self.element = Some(element.into());
        self
    }

    /// Returns the mapping.
    #[must_use]
    pub fn mapping(&self) -> Option<&MappingName> {
        self.mapping.as_ref()
    }

    /// Returns the table.
    #[must_use]
    pub fn table(&self) -> Option<&'static str> {
        self.table
    }

    /// Returns the column.
    #[must_use]
    pub fn column(&self) -> Option<&'static str> {
        self.column
    }

    /// Returns the openEHR element.
    #[must_use]
    pub fn element(&self) -> Option<&str> {
        self.element.as_deref()
    }

    /// Returns why the record was refused.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// A source field no mapping wrote anywhere.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnmappedField {
    mapping: MappingName,
    element: String,
}

impl UnmappedField {
    /// Records that `mapping` wrote the element at the RM path `element`
    /// nowhere.
    #[must_use]
    pub fn new(mapping: MappingName, element: impl Into<String>) -> Self {
        Self {
            mapping,
            element: element.into(),
        }
    }

    /// Returns the mapping.
    #[must_use]
    pub fn mapping(&self) -> &MappingName {
        &self.mapping
    }

    /// Returns the element's RM path.
    #[must_use]
    pub fn element(&self) -> &str {
        &self.element
    }
}

/// The counted outcomes of one composition.
///
/// The engine records the refusals and the unmapped fields as it maps; the
/// writer records what it wrote with [`Report::record_written`]. Every
/// counter is keyed, so a run report can sum them per mapping and in total.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    rows: BTreeMap<(MappingName, &'static str), u64>,
    concept_zero: BTreeMap<(MappingName, &'static str, &'static str), u64>,
    zero_relationship_links: u64,
    unmapped: Vec<UnmappedField>,
    refusals: Vec<Refusal>,
}

impl Report {
    /// Records a refused record.
    pub fn refuse(&mut self, refusal: Refusal) {
        self.refusals.push(refusal);
    }

    /// Records a source field no mapping wrote.
    pub fn unmapped(&mut self, field: UnmappedField) {
        self.unmapped.push(field);
    }

    /// Records that the writer wrote `rows` of `table` for `mapping`, with
    /// `concept_zero` of them holding concept 0 in each named column.
    pub fn record_written(
        &mut self,
        mapping: &MappingName,
        table: &'static str,
        rows: u64,
        concept_zero: &BTreeMap<&'static str, u64>,
    ) {
        let entry = self.rows.entry((mapping.clone(), table)).or_default();
        *entry = entry.saturating_add(rows);
        for (column, count) in concept_zero {
            let entry = self
                .concept_zero
                .entry((mapping.clone(), table, column))
                .or_default();
            *entry = entry.saturating_add(*count);
        }
    }

    /// Records that the writer wrote `rows` `FACT_RELATIONSHIP` rows with
    /// `relationship_concept_id` 0.
    pub fn record_links(&mut self, rows: u64) {
        self.zero_relationship_links = self.zero_relationship_links.saturating_add(rows);
    }

    /// Returns the rows written, by mapping and table.
    #[must_use]
    pub fn rows(&self) -> &BTreeMap<(MappingName, &'static str), u64> {
        &self.rows
    }

    /// Returns the concept-0 assignments, by mapping, table and column.
    #[must_use]
    pub fn concept_zero(&self) -> &BTreeMap<(MappingName, &'static str, &'static str), u64> {
        &self.concept_zero
    }

    /// Returns the `FACT_RELATIONSHIP` rows written with
    /// `relationship_concept_id` 0.
    #[must_use]
    pub fn zero_relationship_links(&self) -> u64 {
        self.zero_relationship_links
    }

    /// Returns the source fields no mapping wrote.
    #[must_use]
    pub fn unmapped_fields(&self) -> &[UnmappedField] {
        &self.unmapped
    }

    /// Returns the refused records.
    #[must_use]
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }
}
