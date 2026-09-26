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
const PENDING: [(&[&str], &str); 2] = [
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
fn etl_run_without_an_etl_section_exits_seventy_eight() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
        .args(["etl", "run"])
        .env_remove("FERROBRIDGE_CONFIG")
        .output()
        .expect("the binary runs");
    assert_eq!(Some(i32::from(EXIT_CONFIG)), output.status.code());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[etl]"),
        "the refusal names the section: {stderr}"
    );
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

/// Runs `ferrobridge serve` with `format` on a port that is already taken, so
/// the boot runs to the bind and stops there, and returns its stdout.
fn serve_until_the_bind_fails(format: &str) -> String {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = taken.local_addr().expect("the bound address");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ferrobridge"))
        .arg("serve")
        .env_remove("FERROBRIDGE_CONFIG")
        .env_remove("RUST_LOG")
        .env("FERROBRIDGE_LOG_FORMAT", format)
        .env("FERROBRIDGE__SERVER__LISTEN", address.to_string())
        .output()
        .expect("the binary runs");
    assert_eq!(Some(1), output.status.code(), "a bind failure is a failure");
    drop(taken);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn serve_under_pretty_prints_the_banner_with_the_version_and_the_pins() {
    let stdout = serve_until_the_bind_fails("pretty");
    assert!(
        stdout.contains(&format!(
            "openEHR bridge to HL7 FHIR and the OMOP CDM · v{}",
            env!("CARGO_PKG_VERSION")
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("Maintained by Ruben Talstra · https://ferrobridge.eu"),
        "{stdout}"
    );
    for pin in ferrobridge_server::build_info::pins() {
        assert!(stdout.contains(&pin.version), "{pin:?}: {stdout}");
    }
    assert!(stdout.contains("facade"), "the lanes are listed: {stdout}");
}

#[test]
fn serve_under_json_prints_no_banner_and_only_json_lines() {
    let stdout = serve_until_the_bind_fails("json");
    assert!(!stdout.contains("Maintained by"), "{stdout}");
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every line is a JSON object"))
        .collect();
    let messages: Vec<&str> = events
        .iter()
        .filter_map(|event| event["message"].as_str())
        .collect();
    assert_eq!(Some(&"console"), messages.first(), "{messages:?}");
    assert!(messages.contains(&"build"), "{messages:?}");
    let lanes: Vec<&str> = events
        .iter()
        .filter(|event| event["message"] == "lane")
        .filter_map(|event| event["lane"].as_str())
        .collect();
    assert_eq!(
        vec!["facade", "hl7v2", "operations", "etl", "terminology"],
        lanes
    );
    let console = &events[0];
    assert_eq!(Some("json"), console["format"].as_str());
    assert_eq!(Some(false), console["colour"].as_bool());
    assert!(console["filter"].as_str().is_some(), "{console}");
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
