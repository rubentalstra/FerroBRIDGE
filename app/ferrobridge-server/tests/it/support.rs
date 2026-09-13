// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Shared helpers: a capturing log writer, test settings and a test state.

use ferrobridge_server::config::ServerSettings;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing_subscriber::fmt::MakeWriter;

/// A `tracing` writer that keeps every line in memory.
#[derive(Debug, Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    /// Returns everything written so far.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// The writer [`Logs`] hands out.
#[derive(Debug)]
pub(crate) struct LogsWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogsWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Logs {
    type Writer = LogsWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogsWriter(Arc::clone(&self.0))
    }
}

/// Returns server settings a test drives the middleware with.
pub(crate) fn settings() -> ServerSettings {
    ServerSettings {
        listen: "127.0.0.1:0".parse().expect("a socket address"),
        request_timeout: Duration::from_secs(5),
        shutdown_timeout: Duration::from_secs(5),
        body_limit: 1024,
    }
}

/// Returns a state with no indicator and no logged query parameter.
pub(crate) fn state() -> Arc<AppState> {
    Arc::new(AppState::with_health(Registry::default()))
}
