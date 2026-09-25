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

/// The jobs that parse and do not run yet, with the issue each names.
const PENDING: [(&[&str], &str); 3] = [
    (&["etl", "run"], "#90"),
    (&["vocab", "load", "/srv/athena", "--schema", "cdm"], "#233"),
    (&["mapping", "check"], "#82"),
];

#[test]
fn every_pending_job_exits_two() {
    for (argv, _) in PENDING {
        let argv: Vec<&str> = std::iter::once("ferrobridge")
            .chain(argv.iter().copied())
            .collect();
        assert_eq!(
            rendered(ExitCode::from(EXIT_UNAVAILABLE_JOB)),
            rendered(run(&argv)),
            "{argv:?} parses and is refused"
        );
    }
}

#[test]
fn the_binary_prints_the_issue_that_lands_each_pending_job() {
    for (argv, issue) in PENDING {
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
fn cdm_init_without_a_cdm_section_exits_seventy_eight() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
        .args(["cdm", "init"])
        .env_remove("FERROBRIDGE_CONFIG")
        .output()
        .expect("the binary runs");
    assert_eq!(Some(i32::from(EXIT_CONFIG)), output.status.code());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[cdm]"),
        "the refusal names the section: {stderr}"
    );
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
