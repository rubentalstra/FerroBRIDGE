// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The console: the format choice, the process-wide subscriber, and the writer
//! that keeps a text record on one line.
//!
//! Two renderings over `tracing`: `pretty` for a person and `json` for a log
//! pipeline, one object per line. `auto` picks `pretty` when stdout is a
//! terminal and `json` otherwise, so a container emits machine lines with no
//! configuration. [`FORMAT_ENV`] and [`FILTER_ENV`] override the
//! `[telemetry]` section. No specification governs the console: our own
//! design.

use serde::Deserialize;
use std::borrow::Cow;
use std::io::{self, Write};
use tracing::{Metadata, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The filter the server runs with when the configuration names none.
///
/// The HTTP stack's own crates are quiet, so the log carries this server's
/// lines. Both [`crate::config::Telemetry`] and the fallback below read it.
pub const DEFAULT_FILTER: &str = "info,hyper=warn,tower=warn,h2=warn";

/// The environment variable that overrides `[telemetry] format`.
pub const FORMAT_ENV: &str = "FERROBRIDGE_LOG_FORMAT";

/// The environment variable that overrides `[telemetry] filter`.
pub const FILTER_ENV: &str = "RUST_LOG";

/// The rendering a deployment asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Format {
    /// `pretty` on a terminal, `json` otherwise.
    #[default]
    Auto,
    /// One JSON object per line.
    Json,
    /// Human-readable lines.
    Pretty,
}

/// The rendering after `auto` is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendering {
    /// One JSON object per line.
    Json,
    /// Human-readable lines.
    Pretty,
}

impl Format {
    /// Decides `auto` from whether stdout is a terminal.
    ///
    /// The caller passes the answer rather than reading it, so a test fixes
    /// the decision without owning a terminal.
    #[must_use]
    pub const fn resolve(self, stdout_is_terminal: bool) -> Rendering {
        match self {
            Self::Pretty => Rendering::Pretty,
            Self::Auto if stdout_is_terminal => Rendering::Pretty,
            Self::Json | Self::Auto => Rendering::Json,
        }
    }

    /// Returns whether the console writes colour.
    ///
    /// An explicit `pretty` keeps its colour into a pipe, because a person
    /// asked for it; `auto` follows the terminal. `json` writes none.
    #[must_use]
    pub const fn colour(self, stdout_is_terminal: bool) -> bool {
        match self {
            Self::Pretty => true,
            Self::Auto => stdout_is_terminal,
            Self::Json => false,
        }
    }

    /// Returns the configuration spelling of this format.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Json => "json",
            Self::Pretty => "pretty",
        }
    }
}

impl Rendering {
    /// Returns the name of this rendering.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Pretty => "pretty",
        }
    }
}

/// A subscriber could not be installed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A subscriber is already installed in this process.
    #[error("a log subscriber is already installed")]
    AlreadyInstalled {
        /// What `tracing-subscriber` reported.
        #[source]
        source: tracing_subscriber::util::TryInitError,
    },
}

/// Builds the subscriber for `rendering` with `filter`, writing through
/// `writer`.
///
/// A filter that does not parse falls back to [`DEFAULT_FILTER`], so a typo in
/// the configuration does not stop the process; [`init`] logs the fallback
/// once the subscriber is running. The `pretty` rendering writes through
/// [`LineSafe`]; the `json` rendering escapes a line break inside a value by
/// construction.
pub fn subscriber<W>(
    rendering: Rendering,
    filter: &str,
    ansi: bool,
    writer: W,
) -> impl Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let layer: Box<dyn tracing_subscriber::Layer<_> + Send + Sync> = match rendering {
        Rendering::Json => Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(writer),
        ),
        Rendering::Pretty => Box::new(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(ansi)
                .with_writer(LineSafe::new(writer)),
        ),
    };
    tracing_subscriber::registry().with(filter).with(layer)
}

/// Installs the process-wide subscriber on stdout, logs the console it runs
/// with, and returns its rendering.
///
/// # Errors
/// Returns [`Error::AlreadyInstalled`] when this process already has one.
pub fn init(format: Format, filter: &str, stdout_is_terminal: bool) -> Result<Rendering, Error> {
    let rendering = format.resolve(stdout_is_terminal);
    let colour = format.colour(stdout_is_terminal);
    subscriber(rendering, filter, colour, io::stdout)
        .try_init()
        .map_err(|source| Error::AlreadyInstalled { source })?;
    let in_effect = if EnvFilter::try_new(filter).is_ok() {
        filter
    } else {
        tracing::warn!(
            filter,
            fallback = DEFAULT_FILTER,
            "the log filter does not parse; using the default"
        );
        DEFAULT_FILTER
    };
    tracing::info!(
        format = format.as_str(),
        rendering = rendering.as_str(),
        filter = in_effect,
        colour,
        "console"
    );
    Ok(rendering)
}

/// Wraps a [`MakeWriter`] so a record it writes carries no interior carriage
/// return or line feed.
///
/// A line break inside a field would let a value forge a second log line, so
/// each one becomes the two characters `\r` or `\n` and the record keeps its
/// one terminating line feed (OWASP Logging Cheat Sheet, §Event collection,
/// <https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html>).
#[derive(Debug, Clone)]
pub struct LineSafe<M>(M);

impl<M> LineSafe<M> {
    /// Wraps `inner`.
    #[must_use]
    pub const fn new(inner: M) -> Self {
        Self(inner)
    }
}

impl<'a, M> MakeWriter<'a> for LineSafe<M>
where
    M: MakeWriter<'a>,
{
    type Writer = LineSafeWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        LineSafeWriter(self.0.make_writer())
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        LineSafeWriter(self.0.make_writer_for(meta))
    }
}

/// The writer [`LineSafe`] hands out.
#[derive(Debug)]
pub struct LineSafeWriter<W>(W);

// NOTE: no specification governs this: our own design. The `fmt` layer writes
// one whole record per call, so the one line feed a record may end in is its own.
impl<W> Write for LineSafeWriter<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(&one_line(buf))?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(&one_line(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Returns `record` with every carriage return and line feed but a final line
/// feed spelled as `\r` and `\n`.
///
/// A record with no interior break comes back borrowed.
fn one_line(record: &[u8]) -> Cow<'_, [u8]> {
    let (body, terminator): (&[u8], &[u8]) = match record.split_last() {
        Some((b'\n', body)) => (body, b"\n"),
        _ => (record, b""),
    };
    if !body.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return Cow::Borrowed(record);
    }
    let mut escaped = Vec::with_capacity(record.len() + 8);
    for byte in body {
        match byte {
            b'\n' => escaped.extend_from_slice(br"\n"),
            b'\r' => escaped.extend_from_slice(br"\r"),
            other => escaped.push(*other),
        }
    }
    escaped.extend_from_slice(terminator);
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FILTER, Format, Rendering, one_line};
    use std::borrow::Cow;
    use tracing_subscriber::EnvFilter;

    #[test]
    fn auto_follows_the_terminal_and_an_explicit_format_does_not() {
        assert_eq!(Rendering::Pretty, Format::Auto.resolve(true));
        assert_eq!(Rendering::Json, Format::Auto.resolve(false));
        assert_eq!(Rendering::Json, Format::Json.resolve(true));
        assert_eq!(Rendering::Pretty, Format::Pretty.resolve(false));
    }

    #[test]
    fn colour_follows_the_terminal_under_auto_and_survives_a_pipe_under_pretty() {
        assert!(Format::Auto.colour(true));
        assert!(!Format::Auto.colour(false));
        assert!(Format::Pretty.colour(false));
        assert!(!Format::Json.colour(true));
    }

    #[test]
    fn the_default_filter_parses_and_quiets_the_http_stack() {
        assert!(EnvFilter::try_new(DEFAULT_FILTER).is_ok());
        assert!(DEFAULT_FILTER.contains("hyper=warn"));
        assert!(DEFAULT_FILTER.contains("tower=warn"));
        assert!(DEFAULT_FILTER.contains("h2=warn"));
    }

    #[test]
    fn an_unparseable_filter_is_refused_by_the_env_filter_so_the_fallback_runs() {
        assert!(
            EnvFilter::try_new("info,=,,").is_err(),
            "the fallback path needs a filter the parser refuses"
        );
    }

    #[test]
    fn an_interior_break_is_escaped_and_the_terminator_is_kept() {
        assert_eq!(
            br"a\r\nb\nc"
                .iter()
                .chain(b"\n")
                .copied()
                .collect::<Vec<u8>>(),
            one_line(b"a\r\nb\nc\n").into_owned()
        );
        assert_eq!(
            br"a\n".to_vec(),
            one_line(b"a\n\n").into_owned()[..3].to_vec()
        );
        assert!(matches!(one_line(b"plain\n"), Cow::Borrowed(_)));
        assert_eq!(br"no\nend".to_vec(), one_line(b"no\nend").into_owned());
    }
}
