// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Arbitrary bytes into the openEHR mapping-path parser, then resolution of
//! the `../` steps against a fixed anchor. An `Err` is the correct answer; a
//! panic is the bug.

#![no_main]

use libfuzzer_sys::fuzz_target;
use openehr_mapping_core::path::MappingPath;
use openehr_rm::v1_2::paths::RmPath;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(path) = source.parse::<MappingPath>() else {
        return;
    };
    let Ok(anchor) = "content[openEHR-EHR-EVALUATION.problem_diagnosis.v1]/data[at0001]".parse::<RmPath>()
    else {
        return;
    };
    let _resolved = path.resolve(&anchor);
});
