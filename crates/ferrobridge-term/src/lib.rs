// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FerroBRIDGE FHIR terminology client for `$lookup`, `$translate` and
//! `$validate-code`.
//!
//! Three operations on one configured server: [`Client::lookup`] resolves a
//! code's display, [`Client::translate`] maps a code into another system, and
//! [`Client::validate_code`] tests membership of a value set.
//! [`Client::batch`] sends several of them as one `batch` `Bundle` and answers
//! them by position.
//!
//! A negative answer an operation states in its own `out` parameters is an
//! outcome ([`outcome`]); an answer the server refused is a typed error
//! carrying the upstream status, the body, and any `tx-issue-type` coding the
//! `OperationOutcome` holds. A missing display is
//! [`LookupOutcome::NotFound`](outcome::LookupOutcome::NotFound), never an
//! empty string.
//!
//! The request and response contracts come from the generated `fhir-types`
//! operation modules; this crate models no FHIR of its own. The release is
//! configured ([`WireVersion`](config::WireVersion)) because a server that
//! serves several releases side by side puts the release in the base path.
#![doc(test(attr(deny(warnings))))]

pub mod client;
pub mod concept;
pub mod config;
mod decode;
pub mod error;
pub mod outcome;
mod wire;

/// The HL7 FHIR release this crate targets.
///
/// The release is published at <https://hl7.org/fhir/R4/>. R4B is accepted on
/// the wire as well ([`WireVersion`](config::WireVersion)).
pub const FHIR_VERSION: &str = "R4";

/// `CodeSystem/$lookup`, as the operation is invoked
/// (<https://hl7.org/fhir/R4/codesystem-operation-lookup.html>).
pub const LOOKUP: &str = "CodeSystem/$lookup";

/// `ConceptMap/$translate`
/// (<https://hl7.org/fhir/R4/conceptmap-operation-translate.html>).
pub const TRANSLATE: &str = "ConceptMap/$translate";

/// `ValueSet/$validate-code`
/// (<https://hl7.org/fhir/R4/valueset-operation-validate-code.html>).
pub const VALIDATE_CODE: &str = "ValueSet/$validate-code";

/// The batch interaction, a `POST` of a `Bundle` to the service base
/// (<https://hl7.org/fhir/R4/http.html#transaction>).
pub const BATCH: &str = "batch";
