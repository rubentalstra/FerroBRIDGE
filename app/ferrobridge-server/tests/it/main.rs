// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests through the library run path the binary shares.

#![allow(
    clippy::panic_in_result_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test assertions"
)]

mod config;
mod http;
mod readiness;
mod run;
mod shutdown;
mod support;
mod telemetry;
