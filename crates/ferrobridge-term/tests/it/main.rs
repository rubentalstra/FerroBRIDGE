// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the three terminology operations against a stub server,
//! the batch form, the transport rules, a run against the reference server in
//! a container, and the crate's pinned specification version.

#![allow(
    clippy::panic_in_result_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test assertions"
)]

mod batch;
mod end_to_end;
mod lookup;
mod pins;
mod support;
mod translate;
mod transport;
mod validate;
