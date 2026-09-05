// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! A bidirectional path model over FHIR JSON guided by the FHIR element
//! table, for mapping engines that write resources as well as read them.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #81 and follows the repository's architecture document.
#![doc(test(attr(deny(warnings))))]

/// The HL7 FHIR release this crate targets.
///
/// The release is published at <https://hl7.org/fhir/R4/>.
pub const FHIR_VERSION: &str = "R4";

// TODO(#81): the implementation this crate name is reserved for.
