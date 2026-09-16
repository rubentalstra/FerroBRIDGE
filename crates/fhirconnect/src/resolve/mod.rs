// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Context resolution: one immutable program per context mapping.
//!
//! A context mapping imports the model mappings and the extensions that map
//! one profile onto one template
//! (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/context-mappings.adoc`).
//! This module compiles that import list once, at load: it applies the
//! extensions, resolves every path on both sides, checks the version
//! selectors, and hands back a [`program::Program`] the interpreter runs per
//! request without parsing a path or reading a file again.
//!
//! The parts are separate modules:
//!
//! - [`program`] is the immutable output and the types it is built from.
//! - [`compile`] is the compiler, a pure function of its inputs.
//! - `extensions` applies `add`, `append` and `overwrite` to a model mapping.
//! - [`select`] picks the program one request runs.
//! - [`error`] holds the diagnostic codes the compiler raises.
//!
//! The specification fixes what each extension method does and leaves open in
//! which order several extensions apply, how their edits collide, and how a
//! version selector is checked, so those rules are FerroBRIDGE's own and are
//! labelled where they are implemented.

pub mod compile;
pub mod error;
mod extensions;
pub mod program;
pub mod select;
