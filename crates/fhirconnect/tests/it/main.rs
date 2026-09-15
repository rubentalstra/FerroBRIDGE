// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the crate's pinned specification version agrees with
//! the pin matrix, the vendored mapping library runs through all three
//! validation layers, and one context of it compiles into one program.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod corpus;
mod engine;
mod operations;
mod pins;
mod resolve;
mod support;
mod tree;
