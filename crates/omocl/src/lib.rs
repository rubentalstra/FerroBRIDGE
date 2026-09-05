// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL mapping language for Rust: the mapping file model and its
//! validation, and the interpreter emitting OMOP CDM rows from openEHR
//! compositions.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #2 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The OMOCL grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://github.com/SevKohler/OMOCL>).
pub const GRAMMAR: &str = "OMOCL/v1.0.0";

// TODO(#2): the implementation this crate name is reserved for.
