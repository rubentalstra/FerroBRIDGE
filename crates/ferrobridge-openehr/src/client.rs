// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The client itself: one configured CDR, one request at a time.

use crate::commit::CommitHeader;
use crate::config::{Config, Credentials};
use crate::error::{Error, UpstreamError};
use crate::ids::RequestId;
use crate::prefer::Prefer;
use backon::{ExponentialBuilder, Retryable};
use http::header::{ACCEPT, CONTENT_TYPE, IF_MATCH, WWW_AUTHENTICATE};
use http::{HeaderMap, Method, StatusCode};
use secrecy::ExposeSecret;
use std::sync::Arc;
use url::Url;

/// The `Prefer` request header name.
const PREFER: &str = "prefer";
/// The correlation header the client echoes when the caller sets one.
const REQUEST_ID: &str = "x-request-id";
/// The canonical JSON media type of ITS-REST 1.1.0.
pub(crate) const CANONICAL_JSON: &str = "application/json";

/// Whether the same request may be sent twice.
///
/// RFC 9110 §9.2.2 makes `GET`, `PUT` and `DELETE` idempotent and `POST` not,
/// so only the first three are ever retried. A `PUT` carries `If-Match` and a
/// `DELETE` addresses a version, so a repeat of either is refused rather than
/// duplicated by the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Idempotency {
    /// The request may be sent again after a transient failure.
    Idempotent,
    /// The request is sent exactly once.
    NonIdempotent,
}

/// A request body and the media type it is sent as.
pub(crate) struct CallBody {
    /// The `Content-Type` of the body.
    pub(crate) content_type: &'static str,
    /// The body itself.
    pub(crate) text: String,
}

/// One request, described so that every attempt builds the same thing.
pub(crate) struct Call {
    /// The HTTP method.
    pub(crate) method: Method,
    /// The absolute URL, query included.
    pub(crate) url: Url,
    /// The `Accept` header value.
    pub(crate) accept: &'static str,
    /// The `Prefer` header value.
    pub(crate) prefer: Prefer,
    /// The `If-Match` header value, already quoted.
    pub(crate) if_match: Option<String>,
    /// The commit metadata headers, in wire order.
    pub(crate) commit_headers: Vec<CommitHeader>,
    /// The request body.
    pub(crate) body: Option<CallBody>,
    /// Whether this request may be retried.
    pub(crate) idempotency: Idempotency,
}

impl Call {
    /// Returns a request of `method` to `url` with no body.
    ///
    /// A read asks for a representation, because ITS-REST 1.1.0 §Requests and
    /// responses/HTTP status codes makes error details conditional on it:
    /// "services MAY return additional error details if the `Prefer:
    /// return=representation` header is present in the request".
    pub(crate) fn new(method: Method, url: Url, idempotency: Idempotency) -> Self {
        Self {
            method,
            url,
            accept: CANONICAL_JSON,
            prefer: Prefer::Representation,
            if_match: None,
            commit_headers: Vec::new(),
            body: None,
            idempotency,
        }
    }

    /// Returns this request with `accept` as its `Accept` header.
    pub(crate) fn accepting(mut self, accept: &'static str) -> Self {
        self.accept = accept;
        self
    }

    /// Returns this request with `prefer` as its `Prefer` header.
    pub(crate) fn preferring(mut self, prefer: Prefer) -> Self {
        self.prefer = prefer;
        self
    }

    /// Returns this request with `text` as a canonical JSON body.
    pub(crate) fn with_json_body(mut self, text: String) -> Self {
        self.body = Some(CallBody {
            content_type: CANONICAL_JSON,
            text,
        });
        self
    }
}

/// What the service answered, read whole.
pub(crate) struct Answer {
    /// The URL that was called.
    pub(crate) url: Url,
    /// The status the service answered with.
    pub(crate) status: StatusCode,
    /// The response headers.
    pub(crate) headers: HeaderMap,
    /// The response body, as received.
    pub(crate) body: String,
}

impl Answer {
    /// Returns the value of response header `name`, when it is usable text.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Returns this answer as an upstream refusal.
    pub(crate) fn upstream(&self) -> UpstreamError {
        UpstreamError::new(self.status, self.body.clone())
    }

    /// Returns the error for a status the operation does not document.
    pub(crate) fn undocumented(&self) -> Error {
        Error::UndocumentedStatus {
            url: self.url.clone(),
            upstream: Box::new(self.upstream()),
        }
    }
}

/// A client for one openEHR CDR.
///
/// The client is cheap to clone: the HTTP stack and the configuration are
/// shared, so a clone carries the same connection pool.
#[derive(Debug, Clone)]
pub struct Client {
    /// The HTTP stack, with the configured timeout.
    http: reqwest::Client,
    /// The CDR this client talks to.
    config: Arc<Config>,
    /// The correlation identifier to echo, when the caller set one.
    request_id: Option<RequestId>,
}

impl Client {
    /// Returns a client for the CDR `config` describes.
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
            request_id: None,
        })
    }

    /// Returns this client with `request_id` echoed into `X-Request-Id`.
    #[must_use]
    pub fn with_request_id(&self, request_id: RequestId) -> Self {
        Self {
            http: self.http.clone(),
            config: Arc::clone(&self.config),
            request_id: Some(request_id),
        }
    }

    /// Returns the configuration this client was built from.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the status the service answers a `GET` on its base URL with.
    ///
    /// The call is sent once, with the configured credentials and timeout, and
    /// every status the service produces is returned as it stands, `401` and
    /// `404` included: the question is whether the service answered, and a
    /// caller reading reachability decides what each status means.
    ///
    /// # Errors
    /// Returns [`Error::Timeout`] when the service did not answer inside the
    /// configured budget and [`Error::Transport`] when the request never
    /// reached it.
    pub async fn reachability(&self) -> Result<StatusCode, Error> {
        let url = self.config.base_url.clone();
        let request = self
            .authorize(self.http.request(Method::GET, url.clone()))
            .header(ACCEPT, CANONICAL_JSON);
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
        tracing::debug!(path = url.path(), status = %status, "the openEHR service answered a probe");
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

    /// Returns the absolute URL of `segments` under the configured base.
    ///
    /// Every segment is percent-encoded, so a template identifier with a space
    /// and a version identifier with `::` both survive.
    pub(crate) fn url(&self, segments: &[&str]) -> Result<Url, Error> {
        let mut url = self.config.base_url.clone();
        {
            let mut path = url.path_segments_mut().map_err(|()| Error::BaseUrl {
                base: self.config.base_url.clone(),
            })?;
            path.pop_if_empty();
            path.extend(segments);
        }
        Ok(url)
    }

    /// Returns the answer to `call`, retrying it when the budget allows.
    pub(crate) async fn execute(&self, call: Call) -> Result<Answer, Error> {
        let client = self;
        let request = &call;
        let attempt = move || async move { client.attempt(request).await };
        match call.idempotency {
            Idempotency::NonIdempotent => attempt().await,
            Idempotency::Idempotent => {
                attempt
                    .retry(self.backoff())
                    .when(Error::is_retryable)
                    .notify(|error: &Error, delay| {
                        tracing::warn!(
                            error = error.kind(),
                            delay_ms = delay.as_millis(),
                            "retrying an idempotent openEHR request"
                        );
                    })
                    .await
            }
        }
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
            .request(call.method.clone(), call.url.clone())
            .header(ACCEPT, call.accept)
            .header(PREFER, call.prefer.as_str());
        if let Some(request_id) = self.request_id.as_ref() {
            request = request.header(REQUEST_ID, request_id.as_str());
        }
        request = self.authorize(request);
        if let Some(if_match) = call.if_match.as_ref() {
            request = request.header(IF_MATCH, if_match);
        }
        for header in &call.commit_headers {
            request = request.header(header.name, header.value.clone());
        }
        if let Some(body) = call.body.as_ref() {
            request = request
                .header(CONTENT_TYPE, body.content_type)
                .body(body.text.clone());
        }

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
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        };
        // The query of a URL can name a subject; the log carries the path only.
        tracing::debug!(
            method = %call.method,
            path = answer.url.path(),
            status = %status,
            "the openEHR service answered"
        );

        if status == StatusCode::UNAUTHORIZED {
            return Err(Error::Unauthorized {
                url: answer.url.clone(),
                challenge: answer.header(WWW_AUTHENTICATE.as_str()).map(str::to_owned),
                upstream: Box::new(answer.upstream()),
            });
        }
        if status.is_server_error() {
            return Err(Error::ServiceFailure {
                url: answer.url.clone(),
                upstream: Box::new(answer.upstream()),
            });
        }
        Ok(answer)
    }
}

/// Returns the `If-Match` field value for `version_id`.
///
/// "The format is always an `version_uid` identifier enclosed by double
/// quotes" (`ehr-codegen.openapi.yaml`, `components.parameters.If-Match`), so
/// the weakness indicator an `ETag` carries never travels back.
pub(crate) fn if_match_value(version_id: &crate::ids::ObjectVersionId) -> String {
    format!("\"{version_id}\"")
}

#[cfg(test)]
mod tests {
    use super::{Client, if_match_value};
    use crate::config::Config;
    use crate::ids::ObjectVersionId;

    fn client() -> Client {
        Client::new(Config::new(
            "http://cdr.invalid/openehr/v1"
                .parse()
                .expect("a valid URL"),
        ))
        .expect("a buildable client")
    }

    #[test]
    fn a_path_segment_is_percent_encoded() {
        let url = client()
            .url(&["definition", "template", "adl1.4", "Vital Signs"])
            .expect("a URL under the base");
        assert_eq!(
            "http://cdr.invalid/openehr/v1/definition/template/adl1.4/Vital%20Signs",
            url.as_str()
        );
    }

    #[test]
    fn a_trailing_slash_on_the_base_does_not_double() {
        let client = Client::new(Config::new(
            "http://cdr.invalid/openehr/v1/"
                .parse()
                .expect("a valid URL"),
        ))
        .expect("a buildable client");
        let url = client.url(&["ehr"]).expect("a URL under the base");
        assert_eq!("http://cdr.invalid/openehr/v1/ehr", url.as_str());
    }

    #[test]
    fn an_if_match_value_is_the_bare_quoted_version_id() {
        let version_id = ObjectVersionId::from_etag("W/\"8849182c::system::1\"")
            .expect("a well-formed version id");
        assert_eq!("\"8849182c::system::1\"", if_match_value(&version_id));
    }

    #[test]
    fn a_request_id_is_carried_by_the_clone() {
        let with_id = client()
            .with_request_id(crate::ids::RequestId::new("corr-1").expect("a printable identifier"));
        assert!(format!("{with_id:?}").contains("corr-1"));
    }
}
