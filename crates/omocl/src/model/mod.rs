// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OMOCL mapping file model and the layers that validate it.
//!
//! A mapping file is read in four steps. The loader in `openehr-mapping-core`
//! parses the YAML, resolves anchors and aliases and reads the header both
//! mapping languages share; [`parse`] lowers the positioned tree into the
//! [`ast`], naming every key it does not know with its position; [`schema`]
//! validates the same document against the JSON Schema FerroBRIDGE authors,
//! since OMOCL publishes none; and [`semantic`] checks the rules a schema
//! cannot express. [`projection`] is the table from an OMOCL key to the CDM
//! columns it writes, which OMOCL states for four of its ten targets, and
//! [`load`] runs all of it over one file or a set.

pub mod ast;
pub mod error;
pub mod load;
pub mod parse;
pub mod projection;
pub mod schema;
pub mod semantic;
