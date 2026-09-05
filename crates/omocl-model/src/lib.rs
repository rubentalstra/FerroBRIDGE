// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL mapping file model: the AST, the authored JSON schema and the
//! projection from OMOCL columns to OMOP CDM columns.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #87 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The OMOCL grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://github.com/SevKohler/OMOCL>).
pub const GRAMMAR: &str = "OMOCL/v1.0.0";

// TODO(#87): the implementation this crate name is reserved for.
