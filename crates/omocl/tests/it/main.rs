// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the crate's pinned grammar agrees with the pin matrix,
//! the vendored mapping library runs through every validation layer, the
//! projection table agrees with the CDM, and every refusal is pinned.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod corpus;
mod domain;
mod engine;
mod lab;
mod pins;
mod projection;
mod refusals;
mod resolve;
mod snapshots;
mod support;
