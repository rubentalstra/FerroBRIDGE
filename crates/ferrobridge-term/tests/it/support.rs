// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared helpers for the contract suites.
//!
//! Every code and canonical URL comes from the testkit fixtures, which are
//! synthetic: no clinical content, no real identifier.

use ferrobridge_term::client::Client;
use ferrobridge_term::config::{Config, RetryPolicy, WireVersion};
use std::error::Error;
use std::time::Duration;
use wiremock::MockServer;

/// The two releases every operation suite runs against.
pub(crate) const RELEASES: [WireVersion; 2] = [WireVersion::R4, WireVersion::R4B];

/// Returns the path segment the reference server serves `version` under.
///
/// That server serves each release under its own prefix, spelled as the
/// lowercased release name, so the configured base URL carries it.
pub(crate) fn release_path(version: WireVersion) -> String {
    version.as_str().to_lowercase()
}

/// Returns a client for `server` in `version`, with a retry budget short
/// enough for a test.
pub(crate) fn client(server: &MockServer, version: WireVersion) -> Result<Client, Box<dyn Error>> {
    Ok(Client::new(config(server, version)?)?)
}

/// Returns the configuration a test client is built from.
pub(crate) fn config(server: &MockServer, version: WireVersion) -> Result<Config, Box<dyn Error>> {
    let base = format!("{}/{}", server.uri(), release_path(version)).parse()?;
    Ok(Config::new(base, version)
        .with_timeout(Duration::from_secs(5))
        .with_retry(RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
        }))
}

/// Returns the values of request header `name` on the request `index` of
/// `server`, in wire order.
pub(crate) async fn request_header(
    server: &MockServer,
    index: usize,
    name: &str,
) -> Result<Vec<String>, Box<dyn Error>> {
    let requests = server
        .received_requests()
        .await
        .ok_or("the mock server is not recording requests")?;
    let request = requests.get(index).ok_or("no request at that index")?;
    Ok(request
        .headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_owned)
        .collect())
}

/// Returns the body of the request `index` of `server`, as JSON.
pub(crate) async fn request_body(
    server: &MockServer,
    index: usize,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let requests = server
        .received_requests()
        .await
        .ok_or("the mock server is not recording requests")?;
    let request = requests.get(index).ok_or("no request at that index")?;
    Ok(serde_json::from_slice(&request.body)?)
}

/// Returns the value of the `Parameters` parameter named `name` in `body`.
///
/// A parameter carries its value in a `value[x]` member, so the member name is
/// returned beside the value.
pub(crate) fn parameter(
    body: &serde_json::Value,
    name: &str,
) -> Option<(String, serde_json::Value)> {
    body.get("parameter")?
        .as_array()?
        .iter()
        .find(|parameter| parameter.get("name").and_then(serde_json::Value::as_str) == Some(name))?
        .as_object()?
        .iter()
        .find(|(key, _)| key.starts_with("value"))
        .map(|(member, value)| (member.clone(), value.clone()))
}
