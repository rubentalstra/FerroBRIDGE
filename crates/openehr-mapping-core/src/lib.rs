// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared foundation of the FerroBRIDGE mapping languages.
//!
//! FHIRconnect and OMOCL are two languages over one header. The FHIRconnect
//! header page states that the header "is standardized for both FHIRconnect
//! and OMOCL"
//! (`docs/specs/fhirconnect/modules/ROOT/pages/basics/main.adoc`, §Header), so
//! this crate models the header once, loads either language's YAML into a
//! positioned value tree, indexes the loaded set by mapping name and by
//! archetype id, and carries one diagnostic type for every refusal. It also
//! carries the openEHR path model the two languages share, including the
//! `../` parent step they add to the openEHR path grammar.
//!
//! Nothing here interprets a mapping. The FHIRconnect and OMOCL crates read
//! the positioned tree this crate returns and apply their own grammar to it.
//!
//! openEHR is a registered trademark of the openEHR Foundation.
#![doc(test(attr(deny(warnings))))]

pub mod composition;
pub mod diagnostic;
pub mod header;
pub mod index;
pub mod loader;
pub mod path;
pub mod position;
pub mod registry;
pub mod schema;
pub mod template;
pub mod value;
