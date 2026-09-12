// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The pin-matrix reader answers from `docs/VERSIONS.md` and refuses an
//! unknown row instead of inventing a value.

use std::error::Error;

#[test]
fn a_known_row_is_read_from_the_matrix() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        ferrobridge_testkit::matrix_pin("FHIR")?,
        "R4",
        "the FHIR row pins R4"
    );
    Ok(())
}

#[test]
fn an_unknown_row_is_an_error() {
    assert!(
        ferrobridge_testkit::matrix_pin("no-such-row-in-the-matrix").is_err(),
        "an absent row must not be read as a value"
    );
}
