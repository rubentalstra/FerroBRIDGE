// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect mapping language for Rust: the mapping file model and its
//! validation, context resolution into an immutable program, and the
//! bidirectional interpreter between openEHR compositions and FHIR resources.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #1 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The FHIRconnect grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/main.html>).
pub const GRAMMAR: &str = "FHIRConnect/v1.0.0";

// TODO(#1): the implementation this crate name is reserved for.
