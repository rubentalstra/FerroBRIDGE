// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! How a client is pointed at one CDR: its base URL, its budget, and its
//! credentials.
//!
//! No specification governs the timeout, the retry budget or the credential
//! form: our own design. ITS-REST 1.1.0 declares `security: []` on every
//! document and leaves authentication to the deployment.

use secrecy::SecretString;
use std::time::Duration;
use url::Url;

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

/// How often, and how far apart, an idempotent call is retried.
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

/// Everything one [`Client`](crate::client::Client) needs about one CDR.
///
/// `base_url` is the openEHR REST API root, the path under which `/ehr`,
/// `/query` and `/definition` live; the ITS-REST 1.1.0 server template is
/// `https://{baseUrl}/v1`, so a deployment URL normally ends in `/v1`.
#[derive(Debug, Clone)]
pub struct Config {
    /// The openEHR REST API root.
    pub base_url: Url,
    /// How long one request may take, connection included.
    pub timeout: Duration,
    /// The retry budget for idempotent calls.
    pub retry: RetryPolicy,
    /// The credentials, when the deployment needs them.
    pub credentials: Option<Credentials>,
}

impl Config {
    /// Returns a configuration for the CDR whose REST API root is `base_url`.
    ///
    /// The timeout and the retry budget take their defaults, and no
    /// credentials are set.
    #[must_use]
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
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
    use super::{Config, Credentials, RetryPolicy};
    use secrecy::SecretString;
    use std::time::Duration;

    #[test]
    fn a_bearer_token_is_not_in_the_debug_rendering() {
        let config = Config::new("http://cdr.invalid/v1".parse().expect("a valid URL"))
            .with_credentials(Credentials::Bearer(SecretString::from("s3cr3t-token")));
        assert!(!format!("{config:?}").contains("s3cr3t-token"));
    }

    #[test]
    fn a_basic_password_is_not_in_the_debug_rendering() {
        let config = Config::new("http://cdr.invalid/v1".parse().expect("a valid URL"))
            .with_credentials(Credentials::Basic {
                user: "bridge".to_owned(),
                password: SecretString::from("s3cr3t-password"),
            });
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("s3cr3t-password"));
        assert!(rendered.contains("bridge"));
    }

    #[test]
    fn the_defaults_are_three_attempts_and_thirty_seconds() {
        let config = Config::new("http://cdr.invalid/v1".parse().expect("a valid URL"));
        assert_eq!(Duration::from_secs(30), config.timeout);
        assert_eq!(RetryPolicy::default(), config.retry);
        assert_eq!(3, config.retry.max_attempts);
    }
}
