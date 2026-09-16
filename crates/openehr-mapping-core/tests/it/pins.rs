// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pin matrix (`docs/VERSIONS.md`) is the single source of truth for every
//! version. This crate builds on the published `openehr-*` crates, so their
//! rows must exist and name one line before the first dependency lands.

use std::error::Error;

#[test]
fn the_openehr_crates_are_pinned_on_one_line() -> Result<(), Box<dyn Error>> {
    let pins = ["openehr-base", "openehr-rm", "openehr-its", "openehr-query"]
        .into_iter()
        .map(ferrobridge_testkit::matrix_pin)
        .collect::<Result<Vec<_>, _>>()?;
    let first = pins
        .first()
        .ok_or("no openehr-* pin rows in docs/VERSIONS.md")?;
    assert!(
        pins.iter().all(|pin| pin == first),
        "the four openehr-* rows in docs/VERSIONS.md name different versions: {pins:?}"
    );
    Ok(())
}
