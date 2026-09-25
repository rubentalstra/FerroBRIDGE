// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL mapping language for Rust: the mapping file model and its
//! validation, and the interpreter emitting OMOP CDM rows from openEHR
//! compositions.
//!
//! [`model`] is the file model: the AST, the JSON Schema FerroBRIDGE authors
//! for OMOCL, the projection from an OMOCL key to the CDM columns it writes,
//! and the rules the schema cannot express. The grammar is OMOCL
//! (<https://github.com/SevKohler/OMOCL>), read from its railroad images and
//! syntax tables, with its mapping library as evidence.
//!
//! openEHR is a registered trademark of the openEHR Foundation.
#![doc(test(attr(deny(warnings))))]

pub mod model;

/// The OMOCL grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://github.com/SevKohler/OMOCL>).
pub const GRAMMAR: &str = "OMOCL/v1.0.0";

// TODO(#90): the engine module, the interpreter emitting CDM record graphs.
