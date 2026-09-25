// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP Common Data Model v5.4 for Rust: generated row types and column
//! metadata, the OHDSI PostgreSQL DDL embedded, the record graph a mapping
//! produces, and the PostgreSQL side that resolves concepts and writes rows.
//!
//! [`generated`] carries one module per CDM table, each with the table's row
//! type and its column metadata, emitted from the OHDSI definitions by
//! `tools/omop-cdm-codegen`; [`meta`] is the metadata vocabulary those modules
//! are written in, [`value`] the three column types Rust has no type for, and
//! [`ddl`] points the embedded DDL at a schema. [`graph`] is the record graph
//! one composition becomes, checked against the metadata as it is built.
//!
//! With the `database` feature, on by default, `database` binds a PostgreSQL
//! pool to a CDM schema and applies the DDL to it, `vocabulary` resolves
//! source codes to standard concepts over the loaded vocabulary tables,
//! `writer` commits one record graph at a time, and `derived` rebuilds the
//! tables the CDM leaves to the ETL. The model is documented at
//! <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
#![doc(test(attr(deny(warnings))))]

// TODO(#233): the vocabulary loader for an Athena export, once #88 records the
// observed file format.

#[cfg(feature = "database")]
pub mod database;
pub mod ddl;
#[cfg(feature = "database")]
pub mod derived;
pub mod generated;
pub mod graph;
pub mod meta;
pub mod value;
#[cfg(feature = "database")]
pub mod vocabulary;
#[cfg(feature = "database")]
pub mod writer;

/// The OMOP Common Data Model version this crate targets.
///
/// The model is documented at <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
pub const CDM_VERSION: &str = "5.4";

/// The tag of `OHDSI/CommonDataModel` the generated layer is emitted from.
///
/// The definitions and the DDL are vendored at this tag
/// (`docs/specs/omop-cdm/PROVENANCE.md`).
pub const CDM_DEFINITIONS_TAG: &str = "v5.4.3";
