// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of the writer and the step each happened at.

use std::fmt;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::lexical::IdError;

use crate::connection::ConnectionError;
use crate::graph;
use crate::graph::key::EmptyIdentifier;
use crate::graph::key::RecordKey;
use crate::graph::key::VisitKey;

/// What the writer was doing when PostgreSQL refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Step {
    /// Creating the bridge schema and its tables.
    Init,
    /// Opening a transaction.
    Begin,
    /// Taking the per-composition lock.
    Lock,
    /// Reading the side table.
    Read,
    /// Deleting the rows an earlier version wrote to a table.
    Delete(&'static str),
    /// Assigning an id from a table's sequence.
    Allocate(&'static str),
    /// Recording the natural keys.
    Record,
    /// Staging and copying the rows of a table.
    Copy(&'static str),
    /// Moving the staged rows of a table into the CDM.
    Insert(&'static str),
    /// Advancing the watermark.
    Watermark,
    /// Rebuilding a derived table.
    Derive(&'static str),
    /// Committing the transaction.
    Commit,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => f.write_str("creating the bridge schema"),
            Self::Begin => f.write_str("opening the transaction"),
            Self::Lock => f.write_str("locking the composition"),
            Self::Read => f.write_str("reading the side table"),
            Self::Delete(table) => write!(f, "deleting the earlier rows of `{table}`"),
            Self::Allocate(table) => write!(f, "assigning an id for `{table}`"),
            Self::Record => f.write_str("recording the natural keys"),
            Self::Copy(table) => write!(f, "copying the rows of `{table}`"),
            Self::Insert(table) => write!(f, "inserting the rows of `{table}`"),
            Self::Watermark => f.write_str("advancing the watermark"),
            Self::Derive(table) => write!(f, "deriving `{table}`"),
            Self::Commit => f.write_str("committing the transaction"),
        }
    }
}

/// A write the CDM database refused or the writer could not complete.
///
/// A refusal inside [`CdmWriter::commit`](crate::writer::CdmWriter::commit)
/// rolls the whole composition back.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// The connection URL was refused before any connection was tried.
    #[error("the CDM database URL is refused")]
    Url(#[from] ConnectionError),
    /// The connection could not be opened.
    #[error("cannot connect to the CDM database")]
    Connect {
        /// What the client reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// PostgreSQL refused a statement.
    #[error("PostgreSQL refused {step}")]
    Database {
        /// What the writer was doing.
        step: Step,
        /// What PostgreSQL reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// The sequence of a table has no id left for a 32-bit key.
    #[error("the id sequence of `{table}` is exhausted")]
    SequenceExhausted {
        /// The table.
        table: &'static str,
        /// What PostgreSQL reported.
        #[source]
        source: tokio_postgres::Error,
    },
    /// A row refers to an EHR that has no `PERSON`, and the policy creates
    /// none.
    #[error(
        "no PERSON is known for the EHR {}, and the policy creates none",
        ehr_id.value()
    )]
    UnknownPerson {
        /// The EHR.
        ehr_id: HierObjectId,
    },
    /// A row refers to a visit the visit derivation has not written.
    #[error(
        "no visit is known for the EHR {} under the source {}",
        key.ehr_id().value(),
        key.source()
    )]
    UnknownVisit {
        /// The visit's key.
        key: VisitKey,
    },
    /// A row refers to a row that neither the graph nor an earlier commit
    /// holds.
    #[error("the row {key} of `{table}` is not known")]
    UnknownRow {
        /// The table.
        table: &'static str,
        /// The key.
        key: Box<RecordKey>,
    },
    /// The CDM metadata has no table the writer names.
    #[error("the CDM metadata refused a table the writer names")]
    Metadata(#[from] graph::row::GraphError),
    /// The side table holds an empty identifier.
    #[error("the side table holds an empty identifier")]
    Identifier(#[from] EmptyIdentifier),
    /// The side table holds a version identifier BASE 1.3 refuses.
    #[error("the side table holds `{value}` as a version, which is no OBJECT_VERSION_ID")]
    Version {
        /// The value found.
        value: String,
        /// What the BASE 1.3 identifier grammar refused.
        #[source]
        source: IdError,
    },
    /// The side table names something this writer never writes.
    #[error("the side table holds `{value}` for {what}, which this writer never writes")]
    SideTable {
        /// What the value should have been.
        what: &'static str,
        /// The value found.
        value: String,
    },
}
