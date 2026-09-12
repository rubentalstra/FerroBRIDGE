// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the generated layer against the vendored OMOP CDM v5.4
//! definitions and OHDSI's rendered DDL, the column types against the forms
//! they accept, and the crate's pinned specification version against the pin
//! matrix.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod ddl;
mod pins;
mod tables;
mod value;
