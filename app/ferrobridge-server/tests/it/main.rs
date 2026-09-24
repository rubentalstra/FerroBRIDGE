// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests through the library run path the binary shares.

#![allow(
    clippy::panic_in_result_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test assertions"
)]
#![allow(
    clippy::indexing_slicing,
    reason = "indexing a serde_json::Value answers Null rather than panicking, and it is how a case reads a response body (<https://docs.rs/serde_json/1/serde_json/enum.Value.html#impl-Index%3CI%3E-for-Value>)"
)]

mod config;
mod facade;
mod facade_e2e;
mod http;
mod lane_templates;
mod operations;
mod readiness;
mod run;
mod shutdown;
mod support;
mod telemetry;
