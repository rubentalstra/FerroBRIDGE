// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The `ferrobridge` binary: a thin entry point over the library run path.

use std::process::ExitCode;

fn main() -> ExitCode {
    ferrobridge_server::run(std::env::args().skip(1))
}
