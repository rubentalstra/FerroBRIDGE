// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How a client is pointed at one terminology server: its versioned base URL,
//! its wire version, its budget, and its credentials.
//!
//! No specification governs the timeout, the retry budget or the credential
//! form: our own design. FHIR leaves authentication to the deployment
//! (<https://hl7.org/fhir/R4/security.html>), so the credentials are optional.

use secrecy::SecretString;
use std::time::Duration;
use url::Url;

/// The FHIR release a server answers in.
///
/// The release decides which generated module reads the `Parameters` and the
/// `OperationOutcome` of a call. The two releases declare the same `in` and
/// `out` parameters for the three operations this client sends, so the choice
/// changes the decoder rather than the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireVersion {
    /// FHIR R4 (<https://hl7.org/fhir/R4/>).
    R4,
    /// FHIR R4B (<https://hl7.org/fhir/R4B/>).
    R4B,
}

impl WireVersion {
    /// Returns the release name as the FHIR specification spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::R4 => "R4",
            Self::R4B => "R4B",
        }
    }
}

/// The credentials the client presents on every request.
///
/// The secret half is a [`SecretString`], so neither a `Debug` rendering nor a
/// log line can carry it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Credentials {
    /// An RFC 6750 bearer token.
    Bearer(SecretString),
    /// RFC 7617 basic authentication.
    Basic {
        /// The user name, which is not a secret.
        user: String,
        /// The password.
        password: SecretString,
    },
}

/// How often, and how far apart, a call is retried.
///
/// The delay grows exponentially from `initial_backoff` and is capped at
/// `max_backoff`; `max_attempts` counts the first try, so `1` disables retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// How many times the request is sent at most, the first try included.
    pub max_attempts: u32,
    /// The delay before the second attempt.
    pub initial_backoff: Duration,
    /// The ceiling every later delay is clamped to.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(5),
        }
    }
}

/// Everything one [`Client`](crate::client::Client) needs about one
/// terminology server.
///
/// `base_url` is the FHIR service base URL, the path `CodeSystem`,
/// `ConceptMap` and `ValueSet` live under
/// (<https://hl7.org/fhir/R4/http.html#root>). A server that serves several
/// releases side by side puts the release in that path, so the base URL and
/// `wire_version` are set together and must agree.
#[derive(Debug, Clone)]
pub struct Config {
    /// The FHIR service base URL.
    pub base_url: Url,
    /// The release the server answers in.
    pub wire_version: WireVersion,
    /// How long one request may take, connection included.
    pub timeout: Duration,
    /// The retry budget.
    pub retry: RetryPolicy,
    /// The credentials, when the deployment needs them.
    pub credentials: Option<Credentials>,
}

impl Config {
    /// Returns a configuration for the server whose base URL is `base_url`.
    ///
    /// The timeout and the retry budget take their defaults, and no
    /// credentials are set.
    #[must_use]
    pub fn new(base_url: Url, wire_version: WireVersion) -> Self {
        Self {
            base_url,
            wire_version,
            timeout: Duration::from_secs(30),
            retry: RetryPolicy::default(),
            credentials: None,
        }
    }

    /// Returns this configuration with `timeout` as its per-request budget.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns this configuration with `retry` as its retry budget.
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Returns this configuration with `credentials` presented on every call.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Credentials, RetryPolicy, WireVersion};
    use secrecy::SecretString;
    use std::time::Duration;

    fn config() -> Config {
        Config::new(
            "http://tx.invalid/r4".parse().expect("a valid URL"),
            WireVersion::R4,
        )
    }

    #[test]
    fn a_bearer_token_is_not_in_the_debug_rendering() {
        let config =
            config().with_credentials(Credentials::Bearer(SecretString::from("s3cr3t-token")));
        assert!(!format!("{config:?}").contains("s3cr3t-token"));
    }

    #[test]
    fn a_basic_password_is_not_in_the_debug_rendering() {
        let config = config().with_credentials(Credentials::Basic {
            user: "bridge".to_owned(),
            password: SecretString::from("s3cr3t-password"),
        });
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("s3cr3t-password"));
        assert!(rendered.contains("bridge"));
    }

    #[test]
    fn the_defaults_are_three_attempts_and_thirty_seconds() {
        let config = config();
        assert_eq!(Duration::from_secs(30), config.timeout);
        assert_eq!(RetryPolicy::default(), config.retry);
        assert_eq!(3, config.retry.max_attempts);
    }

    #[test]
    fn a_wire_version_names_its_release() {
        assert_eq!("R4", WireVersion::R4.as_str());
        assert_eq!("R4B", WireVersion::R4B.as_str());
    }
}
