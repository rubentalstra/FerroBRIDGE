// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `mappingCode` registry a run is handed.

use fhirconnect::engine::traverse::functions::MappingFunctions;
use fhirconnect::engine::traverse::functions::NoMappingFunctions;

#[test]
fn the_registry_this_milestone_ships_holds_nothing() {
    let error = NoMappingFunctions
        .to_openehr("diagnosis", None)
        .expect_err("the registry ships empty");
    assert!(
        error.to_string().contains("diagnosis"),
        "the refusal names the function: {error}"
    );
}
