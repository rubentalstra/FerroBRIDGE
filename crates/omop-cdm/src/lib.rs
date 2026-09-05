// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The OMOP Common Data Model v5.4 for Rust: generated row types and column
//! metadata, the embedded OHDSI DDL, the vocabulary loader and concept
//! resolver.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #73 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The OMOP Common Data Model version this crate targets.
///
/// The model is documented at <https://ohdsi.github.io/CommonDataModel/cdm54.html>.
pub const CDM_VERSION: &str = "5.4";

// TODO(#73): the implementation this crate name is reserved for.
