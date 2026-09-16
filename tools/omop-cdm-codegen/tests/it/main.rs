// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the definitions the generator reads, the model it lowers
//! them to, and the byte-determinism of what it emits.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod definitions;
mod emit;
mod lower;
