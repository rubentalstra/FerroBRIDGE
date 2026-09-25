// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the generated layer against the vendored OMOP CDM v5.4
//! definitions and OHDSI's rendered DDL, the column types against the forms
//! they accept, the record graph against the metadata, the era scripts
//! against the page they follow, the crate's pinned specification version
//! against the pin matrix, and, with the `database` feature, the resolver,
//! the writer and the derived tables against PostgreSQL.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod catalogue;
mod ddl;
#[cfg(feature = "database")]
mod derived;
mod eras;
mod graph;
mod pins;
mod tables;
mod value;
#[cfg(feature = "database")]
mod vocabulary;
#[cfg(feature = "database")]
mod writer;
