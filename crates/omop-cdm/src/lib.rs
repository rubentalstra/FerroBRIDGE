// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP Common Data Model v5.4 for Rust: generated row types and column
//! metadata, and the OHDSI PostgreSQL DDL embedded.
//!
//! [`generated`] carries one module per CDM table, each with the table's row
//! type and its column metadata, emitted from the OHDSI definitions by
//! `tools/omop-cdm-codegen`; [`meta`] is the metadata vocabulary those modules
//! are written in, [`value`] the three column types Rust has no type for, and
//! [`ddl`] points the embedded DDL at a schema. [`database`] binds a
//! PostgreSQL pool to a CDM schema and applies the DDL to it, and
//! [`vocabulary`] resolves source codes to standard concepts over the loaded
//! vocabulary tables. The model is documented at
//! <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
#![doc(test(attr(deny(warnings))))]

// TODO(#233): the vocabulary loader for an Athena export, once #88 records the
// observed file format.

pub mod database;
pub mod ddl;
pub mod generated;
pub mod meta;
pub mod value;
pub mod vocabulary;

/// The OMOP Common Data Model version this crate targets.
///
/// The model is documented at <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
pub const CDM_VERSION: &str = "5.4";

/// The tag of `OHDSI/CommonDataModel` the generated layer is emitted from.
///
/// The definitions and the DDL are vendored at this tag
/// (`docs/specs/omop-cdm/PROVENANCE.md`).
pub const CDM_DEFINITIONS_TAG: &str = "v5.4.3";
