// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How the CDR client is pointed at one CDR: its base URL, its budget, and its
//! credentials, as `[cdr]` resolves them.
//!
//! No specification governs the timeout, the retry budget or the credential
//! form: our own design. ITS-REST 1.1.0 declares `security: []` on every
//! document and leaves authentication to the deployment.

use std::time::Duration;

use openehr_its::rest::client::RetryPolicy;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use url::Url;

/// The credentials the client presents on every request.
///
/// The secret half stays a [`SecretString`] until the client is built, so
/// neither a `Debug` rendering of the settings nor a log line can carry it.
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

impl Credentials {
    /// Returns the credentials the generated-client runtime sends.
    // TODO(#293): hand the secret over as a `SecretString` once the runtime's
    // `Credentials` holds one (openehr-its sibling request).
    pub(crate) fn runtime(&self) -> openehr_its::rest::client::Credentials {
        match self {
            Self::Bearer(token) => {
                openehr_its::rest::client::Credentials::Bearer(token.expose_secret().to_owned())
            }
            Self::Basic { user, password } => openehr_its::rest::client::Credentials::Basic {
                user: user.clone(),
                password: password.expose_secret().to_owned(),
            },
        }
    }
}

/// Everything one [`CdrClient`](crate::cdr::CdrClient) needs about one CDR.
///
/// `base_url` is the openEHR REST API root, the path under which `/ehr`,
/// `/query` and `/definition` live; the ITS-REST 1.1.0 server template is
/// `https://{baseUrl}/v1`, so a deployment URL normally ends in `/v1`.
#[derive(Debug, Clone)]
pub struct CdrConfig {
    /// The openEHR REST API root.
    pub base_url: Url,
    /// How long one request may take, connection included.
    pub timeout: Duration,
    /// The retry budget for idempotent calls.
    pub retry: RetryPolicy,
    /// The credentials, when the deployment needs them.
    pub credentials: Option<Credentials>,
}

impl CdrConfig {
    /// Returns a configuration for the CDR whose REST API root is `base_url`.
    ///
    /// The timeout is 30 seconds, the retry budget is the runtime's default,
    /// and no credentials are set.
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
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns this configuration with `retry` as its retry budget.
    #[must_use]
    pub const fn with_retry(mut self, retry: RetryPolicy) -> Self {
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
    use super::{CdrConfig, Credentials};
    use secrecy::SecretString;
    use std::time::Duration;

    #[test]
    fn a_bearer_token_is_not_in_the_debug_rendering() {
        let config = CdrConfig::new("http://cdr.invalid/v1".parse().expect("a valid URL"))
            .with_credentials(Credentials::Bearer(SecretString::from("s3cr3t-token")));
        assert!(!format!("{config:?}").contains("s3cr3t-token"));
        let runtime = config
            .credentials
            .as_ref()
            .expect("the credentials are set")
            .runtime();
        assert!(!format!("{runtime:?}").contains("s3cr3t-token"));
    }

    #[test]
    fn a_basic_password_is_not_in_the_debug_rendering() {
        let config = CdrConfig::new("http://cdr.invalid/v1".parse().expect("a valid URL"))
            .with_credentials(Credentials::Basic {
                user: "bridge".to_owned(),
                password: SecretString::from("s3cr3t-password"),
            });
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("s3cr3t-password"));
        assert!(rendered.contains("bridge"));
    }

    #[test]
    fn the_default_timeout_is_thirty_seconds() {
        let config = CdrConfig::new("http://cdr.invalid/v1".parse().expect("a valid URL"));
        assert_eq!(Duration::from_secs(30), config.timeout);
        assert_eq!(3, config.retry.max_attempts);
    }
}
