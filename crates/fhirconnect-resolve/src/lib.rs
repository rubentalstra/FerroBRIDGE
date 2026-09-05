// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! FHIRconnect context resolution: a context mapping, its model mappings and
//! its extensions compiled into one immutable program per profile and
//! template.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #83 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The FHIRconnect grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/main.html>).
pub const GRAMMAR: &str = "FHIRConnect/v1.0.0";

// TODO(#83): the implementation this crate name is reserved for.
