// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: both vendored mapping corpora load through the public
//! seam, and the pin matrix names the openEHR crate line this crate consumes.

#![allow(clippy::panic_in_result_fn, reason = "test assertions")]

mod anchors;
mod corpus;
mod malformed;
mod paths;
mod pins;
mod registry;
mod web_template;
