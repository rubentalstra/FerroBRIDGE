// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect mapping file model and the three layers that validate it.
//!
//! A mapping file is read in four steps. The loader in `openehr-mapping-core`
//! parses the YAML and reads the header both mapping languages share;
//! [`parse`] lowers the positioned tree into the [`ast`]; [`schema`] validates
//! the same document against JSON Schema, both the schemas FHIRconnect
//! publishes and the stricter ones this crate ships; and [`semantic`] checks
//! the rules a schema cannot express, such as a cross-file reference that
//! names no loaded mapping.
//!
//! The published schemas and this crate's schemas disagree, deliberately. The
//! published model schema closes a mapping with `additionalProperties: false`
//! while omitting `mappingCode`, `link`, `participationsFunction` and
//! mapping-level `conceptmap`, which the prose defines
//! (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`),
//! so it refuses files the prose admits. [`schema::published`] exists to
//! exercise the published schemas and pin that disagreement; the loader runs
//! [`schema::strict`].

pub mod ast;
pub mod error;
pub mod load;
pub mod parse;
pub mod schema;
pub mod semantic;
