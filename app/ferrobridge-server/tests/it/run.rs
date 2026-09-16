// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binary is thin over the library, so a test drives the real run path.

use ferrobridge_server::{EXIT_CONFIG, EXIT_UNAVAILABLE_JOB};
use std::process::ExitCode;

/// Runs the library entry point with `argv`.
fn run(argv: &[&str]) -> ExitCode {
    ferrobridge_server::run(argv.iter().map(|word| (*word).to_owned()))
}

/// Renders `code` the way `ExitCode` renders itself, so two are comparable.
fn rendered(code: ExitCode) -> String {
    format!("{code:?}")
}

#[test]
fn every_job_but_serve_exits_two_and_names_the_issue_that_lands_it() {
    for argv in [
        ["ferrobridge", "etl", "run"],
        ["ferrobridge", "cdm", "init"],
        ["ferrobridge", "vocab", "load"],
        ["ferrobridge", "mapping", "check"],
    ] {
        assert_eq!(
            rendered(ExitCode::from(EXIT_UNAVAILABLE_JOB)),
            rendered(run(&argv)),
            "{argv:?} parses and is refused"
        );
    }
}

#[test]
fn the_binary_prints_the_issue_that_lands_each_refused_job() {
    for (argv, issue) in [
        (["etl", "run"], "#91"),
        (["cdm", "init"], "#91"),
        (["vocab", "load"], "#91"),
        (["mapping", "check"], "#82"),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
            .args(argv)
            .output()
            .expect("the binary runs");
        assert_eq!(
            Some(i32::from(EXIT_UNAVAILABLE_JOB)),
            output.status.code(),
            "{argv:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(issue), "{argv:?} names {issue}: {stderr}");
        assert_eq!(1, stderr.lines().count(), "one line, not a backtrace");
    }
}

#[test]
fn no_job_at_all_is_a_usage_refusal() {
    assert_eq!(
        rendered(ExitCode::from(EXIT_UNAVAILABLE_JOB)),
        rendered(run(&["ferrobridge"])),
        "the binary does nothing without a job"
    );
}

#[test]
fn help_and_version_are_printed_and_exit_zero() {
    assert_eq!(
        rendered(ExitCode::SUCCESS),
        rendered(run(&["ferrobridge", "--help"]))
    );
    assert_eq!(
        rendered(ExitCode::SUCCESS),
        rendered(run(&["ferrobridge", "--version"]))
    );
}

#[test]
fn a_configuration_file_that_does_not_exist_exits_seventy_eight() {
    assert_eq!(
        rendered(ExitCode::from(EXIT_CONFIG)),
        rendered(run(&[
            "ferrobridge",
            "serve",
            "--config",
            "/nonexistent/ferrobridge.toml"
        ])),
        "a refused configuration is EX_CONFIG, never a generic failure"
    );
}
