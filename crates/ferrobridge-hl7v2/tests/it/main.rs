// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests: MSH-18 decoding, parsing against the generated
//! structures, the acknowledgment on the wire over MLLP, and the v2-to-FHIR
//! `ConceptMap` interpreter over the vendored package, alone and with the
//! crate's shipped supplements over it.

#![allow(
    clippy::panic_in_result_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]

mod condition;
mod corpus;
mod decode;
mod fixtures;
mod grouping;
mod map;
mod parse;
mod supplement;
mod support;
mod wire;
