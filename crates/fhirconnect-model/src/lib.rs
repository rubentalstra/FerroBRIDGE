// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect mapping file model: the AST, the strict schemas and the
//! semantic validation of model, extension and context mappings.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #82 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The FHIRconnect grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/main.html>).
pub const GRAMMAR: &str = "FHIRConnect/v1.0.0";

// TODO(#82): the implementation this crate name is reserved for.
