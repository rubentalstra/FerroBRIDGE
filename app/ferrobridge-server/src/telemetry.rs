// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The console: the format choice and the process-wide subscriber.
//!
//! Two renderings over `tracing`: `pretty` for a person and `json` for a log
//! pipeline, one object per line. `auto` picks `pretty` when stdout is a
//! terminal and `json` otherwise, so a container emits machine lines with no
//! configuration. No specification governs the console: our own design.

use serde::Deserialize;
use std::io;
use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The filter the server runs with when the configuration names none.
///
/// The HTTP stack's own crates are quiet, so the log carries this server's
/// lines. Both [`crate::config::Telemetry`] and the fallback below read it.
pub const DEFAULT_FILTER: &str = "info,hyper=warn,tower=warn,h2=warn";

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
/// once the subscriber is running.
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
                .with_writer(writer),
        ),
    };
    tracing_subscriber::registry().with(filter).with(layer)
}

/// Installs the process-wide subscriber on stdout and returns its rendering.
///
/// # Errors
/// Returns [`Error::AlreadyInstalled`] when this process already has one.
pub fn init(format: Format, filter: &str, stdout_is_terminal: bool) -> Result<Rendering, Error> {
    let rendering = format.resolve(stdout_is_terminal);
    // An explicit `pretty` keeps its colour into a pipe, because a person
    // asked for it; `auto` follows the terminal.
    let ansi = matches!(format, Format::Pretty) || stdout_is_terminal;
    subscriber(rendering, filter, ansi, io::stdout)
        .try_init()
        .map_err(|source| Error::AlreadyInstalled { source })?;
    if EnvFilter::try_new(filter).is_err() {
        tracing::warn!(
            filter,
            fallback = DEFAULT_FILTER,
            "the log filter does not parse; using the default"
        );
    }
    Ok(rendering)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_FILTER, Format, Rendering};
    use tracing_subscriber::EnvFilter;

    #[test]
    fn auto_follows_the_terminal_and_an_explicit_format_does_not() {
        assert_eq!(Rendering::Pretty, Format::Auto.resolve(true));
        assert_eq!(Rendering::Json, Format::Auto.resolve(false));
        assert_eq!(Rendering::Json, Format::Json.resolve(true));
        assert_eq!(Rendering::Pretty, Format::Pretty.resolve(false));
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
}
