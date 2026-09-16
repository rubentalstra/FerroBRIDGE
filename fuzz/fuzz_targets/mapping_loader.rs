// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Arbitrary bytes into the YAML mapping loader: anchors, aliases, merge keys
//! and the header model. An `Err` is the correct answer; a panic is the bug.

#![no_main]

use libfuzzer_sys::fuzz_target;
use openehr_mapping_core::loader;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let _parsed = loader::parse_str("fuzz.yml", source);
    let _loaded = loader::load_str("fuzz.yml", source);
});
