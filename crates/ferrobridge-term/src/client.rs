// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The client itself: one configured terminology server, one request at a
//! time.

use crate::config::{Config, Credentials};
use crate::error::{Error, UpstreamError};
use crate::outcome::{BatchOutcome, LookupOutcome, Request, TranslateOutcome, ValidateOutcome};
use backon::{ExponentialBuilder, Retryable};
use http::header::{ACCEPT, CONTENT_TYPE, WWW_AUTHENTICATE};
use http::{HeaderMap, Method, StatusCode};
use secrecy::ExposeSecret;
use std::sync::Arc;
use url::Url;

/// The FHIR JSON media type (<https://hl7.org/fhir/R4/http.html#mime-type>).
pub const FHIR_JSON: &str = "application/fhir+json";

/// The path of the capability statement a FHIR server publishes
/// (<https://hl7.org/fhir/R4/http.html#capabilities>).
const METADATA: &str = "metadata";

/// One request, described so that every attempt builds the same thing.
struct Call {
    /// The operation, as `Resource/$code`, or the batch marker.
    operation: &'static str,
    /// The absolute URL.
    url: Url,
    /// The FHIR JSON request body.
    body: String,
}

/// What the server answered, read whole.
pub(crate) struct Answer {
    /// The URL that was called.
    pub(crate) url: Url,
    /// The status the server answered with.
    pub(crate) status: StatusCode,
    /// The response body, as received.
    pub(crate) body: String,
}

/// A client for one FHIR terminology server.
///
/// The client is cheap to clone: the HTTP stack and the configuration are
/// shared, so a clone carries the same connection pool.
#[derive(Debug, Clone)]
pub struct Client {
    /// The HTTP stack, with the configured timeout.
    http: reqwest::Client,
    /// The server this client talks to.
    config: Arc<Config>,
}

impl Client {
    /// Returns a client for the server `config` describes.
    ///
    /// This constructor is the only way to obtain a client, so a deployment
    /// that configures no terminology server holds no [`Client`] at all and
    /// the absence is a compile-time `Option<Client>` rather than a runtime
    /// fallback.
    ///
    /// # Errors
    /// Returns [`Error::BaseUrl`] when the base URL cannot carry a path, and
    /// [`Error::ClientBuild`] when the HTTP stack refuses the configuration.
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.base_url.cannot_be_a_base() {
            return Err(Error::BaseUrl {
                base: config.base_url.clone(),
            });
        }
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|source| Error::ClientBuild { source })?;
        Ok(Self {
            http,
            config: Arc::new(config),
        })
    }

    /// Returns the configuration this client was built from.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the status the server answers `GET [base]/metadata` with.
    ///
    /// Every FHIR REST server states its capabilities at that path
    /// (<https://hl7.org/fhir/R4/http.html#capabilities>), so a reachability
    /// question needs no terminology operation. The call is sent once and
    /// every status is returned as it stands, `401` and `404` included.
    ///
    /// # Errors
    /// Returns [`Error::BaseUrl`] when the base URL cannot carry a path,
    /// [`Error::Timeout`] when the server did not answer inside the configured
    /// budget, and [`Error::Transport`] when the request never reached it.
    pub async fn reachability(&self) -> Result<StatusCode, Error> {
        let url = self.url(METADATA)?;
        let request = self
            .authorize(self.http.request(Method::GET, url.clone()))
            .header(ACCEPT, FHIR_JSON);
        let response = request.send().await.map_err(|source| {
            if source.is_timeout() {
                Error::Timeout {
                    url: url.clone(),
                    source,
                }
            } else {
                Error::Transport {
                    url: url.clone(),
                    source,
                }
            }
        })?;
        let status = response.status();
        tracing::debug!(path = url.path(), status = %status, "the terminology server answered a probe");
        Ok(status)
    }

    /// Returns `request` carrying the configured credentials, when there are
    /// any.
    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.config.credentials.as_ref() {
            None => request,
            Some(Credentials::Bearer(token)) => request.bearer_auth(token.expose_secret()),
            Some(Credentials::Basic { user, password }) => {
                request.basic_auth(user, Some(password.expose_secret()))
            }
        }
    }

    /// Resolves the display of `code` in `system`.
    ///
    /// A server that does not know the code answers [`LookupOutcome::NotFound`]
    /// carrying its `OperationOutcome`; the display is never an empty string.
    ///
    /// # Errors
    /// Returns [`Error::Refused`] when the server refused the operation for
    /// another reason, and [`Error`] otherwise when the call did not reach an
    /// answer or the answer could not be decoded.
    pub async fn lookup(
        &self,
        system: &str,
        code: &str,
        version: Option<&str>,
    ) -> Result<LookupOutcome, Error> {
        let request = Request::Lookup {
            system: system.to_owned(),
            code: code.to_owned(),
            version: version.map(str::to_owned),
        };
        let answer = self.operation(&request).await?;
        crate::decode::lookup(self.config.wire_version, &answer)
    }

    /// Translates `code` from `system` into `target_system`.
    ///
    /// Every match the server returned is carried with its equivalence, so the
    /// caller decides which ones count;
    /// [`TranslateOutcome::accepted`] yields the `equivalent` and `equal` ones.
    ///
    /// # Examples
    ///
    /// A deployment with no terminology server holds no client, and a
    /// translation it cannot make is a refusal:
    ///
    /// ```rust,no_run
    /// use ferrobridge_term::client::Client;
    ///
    /// # fn main() {
    /// # let terminology: Option<Client> = None;
    /// # let _call = async move {
    /// let Some(client) = terminology.as_ref() else {
    ///     return Err("no terminology server is configured".into());
    /// };
    /// let source = "http://example.org/source";
    /// let target = "http://example.org/target";
    /// let outcome = client.translate(source, "alpha", target, None).await?;
    /// let Some(translated) = outcome.accepted().next() else {
    ///     return Err("no equivalent translation".into());
    /// };
    /// let code = translated.concept.as_ref().and_then(|concept| concept.code.as_deref());
    /// assert!(code.is_some(), "an accepted match names its target code");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # };
    /// # }
    /// ```
    ///
    /// # Errors
    /// Returns [`Error::Refused`] when the server refused the operation, for
    /// example because it serves no such concept map, and [`Error`] otherwise
    /// when the call did not reach an answer or the answer could not be
    /// decoded. A refusal is never a missing translation.
    pub async fn translate(
        &self,
        system: &str,
        code: &str,
        target_system: &str,
        concept_map_url: Option<&str>,
    ) -> Result<TranslateOutcome, Error> {
        let request = Request::Translate {
            system: system.to_owned(),
            code: code.to_owned(),
            target_system: target_system.to_owned(),
            concept_map_url: concept_map_url.map(str::to_owned),
        };
        let answer = self.operation(&request).await?;
        crate::decode::translate(self.config.wire_version, &answer)
    }

    /// Validates that `code` from `system` is a member of `value_set_url`.
    ///
    /// # Errors
    /// Returns [`Error::Refused`] when the server refused the operation, for
    /// example because it serves no such value set, and [`Error`] otherwise
    /// when the call did not reach an answer or the answer could not be
    /// decoded. A refusal is never an invalid code.
    pub async fn validate_code(
        &self,
        value_set_url: &str,
        system: &str,
        code: &str,
    ) -> Result<ValidateOutcome, Error> {
        let request = Request::ValidateCode {
            value_set_url: value_set_url.to_owned(),
            system: system.to_owned(),
            code: code.to_owned(),
        };
        let answer = self.operation(&request).await?;
        crate::decode::validate_code(self.config.wire_version, &answer)
    }

    /// Sends `requests` as one `batch` `Bundle` and answers them by position.
    ///
    /// "The server processes each entry independently" and answers a
    /// `batch-response` whose entries are "in the same order as the request"
    /// (<https://hl7.org/fhir/R4/http.html#transaction>), so entry `n` is the
    /// answer to request `n`. An entry the server refused is that entry's own
    /// typed error and leaves the others alone.
    ///
    /// # Errors
    /// Returns [`Error::BatchArity`] when the answer holds a different number
    /// of entries than were sent, and [`Error`] otherwise when the batch
    /// itself did not reach an answer or its `Bundle` could not be decoded.
    pub async fn batch(
        &self,
        requests: &[Request],
    ) -> Result<Vec<Result<BatchOutcome, Error>>, Error> {
        let version = self.config.wire_version;
        let body =
            crate::wire::batch_body(version, requests).map_err(|source| Error::RequestBody {
                operation: crate::BATCH,
                source,
            })?;
        let answer = self
            .execute(Call {
                operation: crate::BATCH,
                url: self.config.base_url.clone(),
                body,
            })
            .await?;
        if answer.status != StatusCode::OK {
            return Err(self.refusal(crate::BATCH, &answer));
        }
        let entries =
            crate::wire::batch_entries(version, &answer.body).map_err(|source| Error::Body {
                operation: crate::BATCH,
                url: answer.url.clone(),
                source: Box::new(source),
            })?;
        if entries.len() != requests.len() {
            return Err(Error::BatchArity {
                sent: requests.len(),
                answered: entries.len(),
            });
        }
        Ok(requests
            .iter()
            .zip(entries)
            .map(|(request, entry)| {
                let answer = Answer {
                    url: answer.url.clone(),
                    status: entry.status,
                    body: entry.body,
                };
                crate::decode::batch_entry(version, request, &answer)
            })
            .collect())
    }

    /// Returns the answer to the `POST` form of `request`.
    ///
    /// The operations are invoked with a `Parameters` body, the form that
    /// carries every input whatever its type
    /// (<https://hl7.org/fhir/R4/operations.html>).
    async fn operation(&self, request: &Request) -> Result<Answer, Error> {
        let operation = request.operation();
        let body = crate::wire::request_body(self.config.wire_version, request)
            .map_err(|source| Error::RequestBody { operation, source })?;
        let url = self.url(operation)?;
        self.execute(Call {
            operation,
            url,
            body,
        })
        .await
    }

    /// Returns the absolute URL of `operation` under the configured base.
    fn url(&self, operation: &str) -> Result<Url, Error> {
        let mut url = self.config.base_url.clone();
        {
            let mut path = url.path_segments_mut().map_err(|()| Error::BaseUrl {
                base: self.config.base_url.clone(),
            })?;
            path.pop_if_empty();
            path.extend(operation.split('/'));
        }
        Ok(url)
    }

    /// Returns the answer to `call`, retrying it when the budget allows.
    async fn execute(&self, call: Call) -> Result<Answer, Error> {
        let client = self;
        let request = &call;
        let attempt = move || async move { client.attempt(request).await };
        attempt
            .retry(self.backoff())
            .when(Error::is_retryable)
            .notify(|error: &Error, delay| {
                tracing::warn!(
                    error = error.kind(),
                    delay_ms = delay.as_millis(),
                    "retrying a terminology request"
                );
            })
            .await
    }

    /// Returns the retry schedule the configuration asks for.
    fn backoff(&self) -> ExponentialBuilder {
        let retries =
            usize::try_from(self.config.retry.max_attempts.saturating_sub(1)).unwrap_or(usize::MAX);
        ExponentialBuilder::new()
            .with_min_delay(self.config.retry.initial_backoff)
            .with_max_delay(self.config.retry.max_backoff)
            .with_max_times(retries)
    }

    /// Sends `call` once and reads the whole answer.
    async fn attempt(&self, call: &Call) -> Result<Answer, Error> {
        let mut request = self
            .http
            .request(Method::POST, call.url.clone())
            .header(ACCEPT, FHIR_JSON)
            .header(CONTENT_TYPE, FHIR_JSON)
            .body(call.body.clone());
        request = self.authorize(request);

        let response = request.send().await.map_err(|source| {
            if source.is_timeout() {
                Error::Timeout {
                    url: call.url.clone(),
                    source,
                }
            } else {
                Error::Transport {
                    url: call.url.clone(),
                    source,
                }
            }
        })?;

        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(|source| Error::Transport {
            url: call.url.clone(),
            source,
        })?;
        let answer = Answer {
            url: call.url.clone(),
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        };
        // A code is a clinical fact, so the log carries the path and never the
        // body or the query.
        tracing::debug!(
            operation = call.operation,
            path = answer.url.path(),
            status = %status,
            "the terminology server answered"
        );

        if status == StatusCode::UNAUTHORIZED {
            return Err(Error::Unauthorized {
                url: answer.url.clone(),
                challenge: challenge(&headers),
                upstream: Box::new(self.upstream(&answer)),
            });
        }
        if status.is_server_error() {
            return Err(Error::ServerFailure {
                url: answer.url.clone(),
                upstream: Box::new(self.upstream(&answer)),
            });
        }
        Ok(answer)
    }

    /// Returns `answer` as an upstream refusal, read in the wire version.
    fn upstream(&self, answer: &Answer) -> UpstreamError {
        UpstreamError::new(self.config.wire_version, answer.status, answer.body.clone())
    }

    /// Returns the error for an answer the operation cannot read.
    fn refusal(&self, operation: &'static str, answer: &Answer) -> Error {
        Error::Refused {
            operation,
            url: answer.url.clone(),
            upstream: Box::new(self.upstream(answer)),
        }
    }
}

/// Returns the `WWW-Authenticate` challenge, when the answer carries usable
/// text.
fn challenge(headers: &HeaderMap) -> Option<String> {
    headers
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::Client;
    use crate::config::{Config, WireVersion};

    fn client(base: &str) -> Client {
        Client::new(Config::new(
            base.parse().expect("a valid URL"),
            WireVersion::R4,
        ))
        .expect("a buildable client")
    }

    #[test]
    fn an_operation_url_sits_under_the_versioned_base() {
        let url = client("http://tx.invalid/fhir/r4")
            .url("CodeSystem/$lookup")
            .expect("a URL under the base");
        assert_eq!("http://tx.invalid/fhir/r4/CodeSystem/$lookup", url.as_str());
    }

    #[test]
    fn a_trailing_slash_on_the_base_does_not_double() {
        let url = client("http://tx.invalid/r4b/")
            .url("ValueSet/$validate-code")
            .expect("a URL under the base");
        assert_eq!(
            "http://tx.invalid/r4b/ValueSet/$validate-code",
            url.as_str()
        );
    }
}
