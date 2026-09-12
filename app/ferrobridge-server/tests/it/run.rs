// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The binary is thin over the library, so a test drives the real run path.

use std::process::ExitCode;

#[test]
fn the_run_path_exits_successfully_with_no_arguments() {
    let code = ferrobridge_server::run(Vec::<String>::new());
    assert_eq!(
        format!("{code:?}"),
        format!("{:?}", ExitCode::SUCCESS),
        "the skeleton run path should exit successfully"
    );
}
