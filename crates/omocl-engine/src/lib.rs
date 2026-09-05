// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL interpreter emitting record graphs of typed OMOP CDM rows from openEHR compositions.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #90 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The OMOCL grammar this crate implements.
///
/// A mapping file writes it in its `grammar` header (<https://github.com/SevKohler/OMOCL>).
pub const GRAMMAR: &str = "OMOCL/v1.0.0";

// TODO(#90): the implementation this crate name is reserved for.
