// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The console: the rendering `auto` resolves to, and the filter fallback.

use crate::support::Logs;
use ferrobridge_server::telemetry::{DEFAULT_FILTER, Format, Rendering, subscriber};

/// Writes one `info` line through a subscriber built for `rendering` and
/// `filter`, and returns what it wrote.
fn emitted(rendering: Rendering, filter: &str) -> String {
    let logs = Logs::default();
    let capture = subscriber(rendering, filter, false, logs.clone());
    tracing::subscriber::with_default(capture, || {
        tracing::info!(subject = "synthetic", "a line");
    });
    logs.text()
}

#[test]
fn auto_renders_json_off_a_terminal_and_pretty_on_one() {
    assert_eq!(Rendering::Json, Format::Auto.resolve(false));
    assert_eq!(Rendering::Pretty, Format::Auto.resolve(true));
}

#[test]
fn an_explicit_rendering_ignores_the_terminal() {
    assert_eq!(Rendering::Json, Format::Json.resolve(true));
    assert_eq!(Rendering::Pretty, Format::Pretty.resolve(false));
}

#[test]
fn the_json_rendering_writes_one_object_per_line() {
    let text = emitted(Rendering::Json, DEFAULT_FILTER);
    let line = text.lines().next().expect("one line was written");
    let document: serde_json::Value =
        serde_json::from_str(line).expect("the line is a JSON object");
    assert_eq!(Some("a line"), document["message"].as_str());
    assert_eq!(Some("synthetic"), document["subject"].as_str());
}

#[test]
fn the_pretty_rendering_writes_readable_text() {
    let text = emitted(Rendering::Pretty, DEFAULT_FILTER);
    assert!(text.contains("a line"), "{text}");
    assert!(
        serde_json::from_str::<serde_json::Value>(text.lines().next().unwrap_or_default()).is_err(),
        "the pretty rendering is not JSON: {text}"
    );
}

#[test]
fn a_filter_that_does_not_parse_falls_back_to_the_default_instead_of_dropping_the_line() {
    let text = emitted(Rendering::Json, "info,=,,");
    assert!(
        text.contains("a line"),
        "the fallback filter still admits an info line: {text}"
    );
}

#[test]
fn a_filter_that_parses_is_honoured() {
    let text = emitted(Rendering::Json, "error");
    assert!(text.is_empty(), "an info line is below error: {text}");
}

/// A value carrying a line break and a forged record of its own.
const FORGED: &str = "ok\r\n2026-09-25T00:00:00Z ERROR forged: a second record";

/// Writes one `warn` line carrying [`FORGED`] as a field and inside the
/// message, and returns what the subscriber for `rendering` wrote.
fn emitted_forged(rendering: Rendering) -> String {
    let logs = Logs::default();
    let capture = subscriber(rendering, DEFAULT_FILTER, false, logs.clone());
    tracing::subscriber::with_default(capture, || {
        tracing::warn!(value = %FORGED, "a value reached a field: {FORGED}");
    });
    logs.text()
}

#[test]
fn a_line_feed_in_a_field_cannot_forge_a_second_pretty_line() {
    let text = emitted_forged(Rendering::Pretty);
    assert_eq!(1, text.lines().count(), "one record, one line: {text:?}");
    assert!(
        text.ends_with('\n'),
        "the record keeps its terminator: {text:?}"
    );
    assert!(
        text.contains(r"ok\r\n2026-09-25T00:00:00Z ERROR forged"),
        "the break is spelled out, every character kept: {text:?}"
    );
}

#[test]
fn a_line_feed_in_a_field_stays_escaped_in_the_json_line() {
    let text = emitted_forged(Rendering::Json);
    assert_eq!(1, text.lines().count(), "one object, one line: {text:?}");
    let document: serde_json::Value =
        serde_json::from_str(text.trim_end()).expect("the line is a JSON object");
    assert_eq!(Some(FORGED), document["value"].as_str());
}
