// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: the ITS-REST 1.1.0 contract against a stub CDR, and the
//! crate's pinned specification version against the pin matrix.

#![allow(
    clippy::panic_in_result_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test assertions"
)]

mod composition;
mod contribution;
mod ehr;
mod pins;
mod query;
mod support;
mod template;
mod transport;
