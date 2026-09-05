// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The pin matrix (`docs/VERSIONS.md`) is the single source of truth for every
//! version; the constant this crate exposes must agree with it.

use std::error::Error;

#[test]
fn the_constant_matches_the_pin_matrix() -> Result<(), Box<dyn Error>> {
    let pin = ferrobridge_testkit::matrix_pin("openEHR ITS-REST")?;
    assert_eq!(
        Some(openehr_path::ITS_REST_VERSION),
        Some(pin.as_str()),
        "the crate constant and the docs/VERSIONS.md row for openEHR ITS-REST disagree"
    );
    Ok(())
}
