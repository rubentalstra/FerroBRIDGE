// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the crate's pinned specification version agrees with
//! the pin matrix, and the vendored mapping library runs through all three
//! validation layers.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod corpus;
mod pins;
