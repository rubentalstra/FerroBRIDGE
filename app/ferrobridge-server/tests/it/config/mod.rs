// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The configuration contract: the file, the environment over it, the `_file`
//! secrets, and every refusal.

mod environment;
mod facade;
mod file;
mod hl7v2;
mod refusals;
mod secrets;

use ferrobridge_server::config::{Config, Error};
use std::collections::BTreeMap;
use std::error::Error as StdError;

/// A file with every section set, so a test can override one key at a time.
const FULL: &str = r#"
[server]
listen = "0.0.0.0:9000"
request_timeout_ms = 1000
shutdown_timeout_ms = 2000
body_limit_bytes = 4096

[telemetry]
format = "json"
filter = "debug"
logged_query_parameters = ["_count"]

[cdr]
base_url = "http://cdr.invalid/v1"
timeout_ms = 5000

[cdr.retry]
max_attempts = 5
initial_backoff_ms = 10
max_backoff_ms = 20

[terminology]
base_url = "http://tx.invalid/r4"
wire_version = "r4b"

[cdm]
url = "postgres://bridge@db.invalid/cdm?sslmode=disable"

[mappings]
directory = "/srv/ferrobridge/mappings"
templates = "/srv/ferrobridge/templates"

[operations]
enabled = false
device_reference = "Device/bridge-one"
composer = "A synthetic composer"
composition_language = "de"
composition_territory = "DE"
"#;

/// Returns the environment map a single override makes.
fn env(name: &str, value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}

/// Returns the refusal `text` resolves to.
fn refusal(text: &str) -> Result<Error, Box<dyn StdError>> {
    Ok(Config::from_sources(Some(text), &BTreeMap::new())?
        .resolve()
        .expect_err("the configuration is refused"))
}
