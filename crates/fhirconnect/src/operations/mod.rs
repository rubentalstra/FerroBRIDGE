// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two FHIRconnect operations, `$tofhir` and `$toopenehr`.
//!
//! The FHIRconnect REST API chapter defines one operation per mapping
//! direction, both `POST` against the service base, both pure
//! transformations, and both following the FHIR R4 operations framework
//! (<https://hl7.org/fhir/R4/operations.html>). The chapter is an unmerged
//! draft, pull request #93 of the specification, vendored at its pinned
//! commit under `docs/specs/fhirconnect/draft-rest-api/`; every wire shape
//! here is read from that text and is re-adjudicated when the chapter merges.
//!
//! The parts are separate modules:
//!
//! - [`contract`] is the typed request and response of each operation, with
//!   the `Parameters` reader and writer the framework asks for.
//! - [`programs`] is the compiled set one call selects from.
//! - [`run`] runs the engine behind each operation.
//! - [`provenance`] builds the `Provenance` every `$tofhir` run carries.
//! - [`issues`] renders the engine's declared losses as `OperationOutcome`
//!   issues.
//! - [`error`] is the typed refusal, with the R4 issue code each one renders
//!   as.
//!
//! Where the draft leaves room, FerroBRIDGE pins its own answer and says so:
//! strictness is the default, so a failed mapping answers an
//! `OperationOutcome` and no Bundle or composition; the Bundle type is
//! `collection`; and a Bundle that references more than one subject is a
//! refusal, because one Bundle maps to one composition.

pub mod contract;

pub mod error;

pub mod issues;

pub mod programs;

pub mod provenance;

pub mod run;

/// The `$tofhir` operation code, without the `$`.
///
/// `ToFhir.fsh` declares `code = #tofhir` at the system level.
pub const TO_FHIR: &str = "tofhir";

/// The `$toopenehr` operation code, without the `$`.
///
/// `ToOpenEhr.fsh` declares `code = #toopenehr` at the system level.
pub const TO_OPENEHR: &str = "toopenehr";

/// The media type the draft defines for an openEHR payload.
///
/// "This specification defines the media type `application/openehr+json` to
/// identify a payload as openEHR canonical JSON", with the suffix ordering of
/// RFC 6839 §4 (<https://www.rfc-editor.org/rfc/rfc6839#section-4>).
pub const OPENEHR_JSON: &str = "application/openehr+json";
