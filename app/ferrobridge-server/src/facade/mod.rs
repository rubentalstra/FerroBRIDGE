// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR R4 REST facade over the openEHR CDR.
//!
//! FerroBRIDGE stores no clinical data of its own: every interaction here maps
//! onto CDR operations through a compiled FHIRconnect program
//! (`docs/architecture.md` §4.6). What the facade keeps is identity, in the
//! store of [`identity`], so a re-sent resource updates the composition it
//! already produced and a read resolves a FHIR id back to one entry.
//!
//! Two rules run through every module. A status is a `StatusCode`, mapped from
//! the CDR's own outcome by the one table in [`status`]. Everything the facade
//! authors is an `OperationOutcome`, and an upstream openEHR error body
//! travels inside `issue.diagnostics` rather than reaching the wire raw.

pub mod identity;
pub mod media;
pub mod outcome;
pub mod status;
