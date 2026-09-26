// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The record graph: the CDM rows one composition becomes, keyed by where
//! they came from rather than by a surrogate id.
//!
//! The OMOCL engine emits a [`RecordGraph`] per composition and the CDM
//! writer commits it whole. Every CDM v5.4 primary key is a 32-bit
//! `integer`, so the ids are the writer's to assign: a [`Row`] carries a
//! [`RecordKey`] (the EHR, the versioned composition, the archetype root and
//! the occurrence) instead, and a column that names another row carries a
//! [`Reference`](crate::graph::row::Reference) the writer resolves. A
//! [`Link`] is a `FACT_RELATIONSHIP` between two rows of the graph. The
//! [`Report`] carries the outcomes the engine counts, and the writer completes
//! it with what it wrote.
//!
//! A row is checked against the generated column metadata as it is built, so
//! a graph that exists names only real columns, holds only values their CDM
//! type admits, and carries every required column. No specification governs
//! this shape: our own design (the CDM leaves keys and provenance to the
//! ETL, <https://ohdsi.github.io/CommonDataModel/cdm54.html>).

pub mod key;
pub mod link;
pub mod report;
pub mod row;

use std::collections::BTreeSet;

use crate::graph::key::RecordKey;
use crate::graph::key::Source;
use crate::graph::link::Link;
use crate::graph::report::Report;
use crate::graph::row::GraphError;
use crate::graph::row::Row;

/// The rows and links one composition version becomes.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordGraph {
    source: Source,
    rows: Vec<Row>,
    links: Vec<Link>,
    keys: BTreeSet<(&'static str, RecordKey)>,
    report: Report,
}

impl RecordGraph {
    /// Starts the empty graph of `source`.
    #[must_use]
    pub fn new(source: Source) -> Self {
        Self {
            source,
            rows: Vec::new(),
            links: Vec::new(),
            keys: BTreeSet::new(),
            report: Report::default(),
        }
    }

    /// Adds `row`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::ForeignRow`] when the row's key names another
    /// EHR or versioned composition than the graph's source, and
    /// [`GraphError::DuplicateRow`] when the table already holds a row under
    /// the key.
    pub fn push_row(&mut self, row: Row) -> Result<(), GraphError> {
        let key = row.key();
        if key.ehr_id() != self.source.ehr_id()
            || key.versioned_object_uid() != self.source.versioned_object_uid()
        {
            return Err(GraphError::ForeignRow {
                table: row.table().name,
                key: Box::new(key.clone()),
            });
        }
        if !self.keys.insert((row.table().name, key.clone())) {
            return Err(GraphError::DuplicateRow {
                table: row.table().name,
                key: Box::new(key.clone()),
            });
        }
        self.rows.push(row);
        Ok(())
    }

    /// Adds `link`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::DanglingLink`] when an end names a row the graph
    /// does not hold; push the rows first.
    pub fn push_link(&mut self, link: Link) -> Result<(), GraphError> {
        for end in [link.first(), link.second()] {
            if !self.keys.contains(&(end.table().name, end.key().clone())) {
                return Err(GraphError::DanglingLink {
                    table: end.table().name,
                    key: Box::new(end.key().clone()),
                });
            }
        }
        self.links.push(link);
        Ok(())
    }

    /// Returns the composition version the graph was mapped from.
    #[must_use]
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Returns the rows, in the order they were added.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Returns the links, in the order they were added.
    #[must_use]
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Returns the report.
    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Returns the report, for the engine to record into.
    pub fn report_mut(&mut self) -> &mut Report {
        &mut self.report
    }

    /// Returns the report, consuming the graph.
    #[must_use]
    pub fn into_report(self) -> Report {
        self.report
    }
}

/// Returns the bound of a `varchar(n)` datatype, `None` for any other.
fn varchar_limit(datatype: &str) -> Option<usize> {
    // NOTE: no specification governs this: our own design; `varchar(MAX)` has
    // no number, so a parse failure is the answer "unbounded".
    datatype
        .strip_prefix("varchar(")?
        .strip_suffix(')')?
        .parse()
        .ok()
}
