// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP Common Data Model v5.4 for Rust: generated row types and column
//! metadata, and the OHDSI PostgreSQL DDL embedded.
//!
//! [`generated`] carries one module per CDM table, each with the table's row
//! type and its column metadata, emitted from the OHDSI definitions by
//! `tools/omop-cdm-codegen`; [`meta`] is the metadata vocabulary those modules
//! are written in, [`value`] the three column types Rust has no type for, and
//! [`ddl`] points the embedded DDL at a schema. The model is documented at
//! <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
//!
//! The vocabulary loader and the concept resolver follow in a later increment.
#![doc(test(attr(deny(warnings))))]

pub mod ddl;
pub mod generated;
pub mod meta;
pub mod value;

/// The OMOP Common Data Model version this crate targets.
///
/// The model is documented at <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
pub const CDM_VERSION: &str = "5.4";

/// The tag of `OHDSI/CommonDataModel` the generated layer is emitted from.
///
/// The definitions and the DDL are vendored at this tag
/// (`docs/specs/omop-cdm/PROVENANCE.md`).
pub const CDM_DEFINITIONS_TAG: &str = "v5.4.3";
