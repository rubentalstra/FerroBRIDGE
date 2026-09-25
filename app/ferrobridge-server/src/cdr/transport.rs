// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The engine one call sends through: the configured `reqwest` engine, plus
//! the headers the generated parameters do not carry and a record of the last
//! answer.
//!
//! No specification governs this: our own design. The generated client reads
//! a documented refusal into a variant that drops its body, so the engine keeps
//! the answer the facade's status table diagnoses.

use std::sync::Mutex;
use std::sync::PoisonError;

use http::HeaderMap;
use openehr_its::rest::client::ReqwestTransport;
use openehr_its::rest::client::Transport;
use openehr_its::rest::client::TransportError;

use crate::cdr::error::Upstream;

/// The engine of one call.
#[derive(Debug)]
pub(crate) struct Scoped<'t> {
    /// The configured engine, with its pool and its timeout.
    inner: &'t ReqwestTransport,
    /// The headers every attempt carries beside the generated ones.
    headers: HeaderMap,
    /// The last answer an attempt read.
    answered: Mutex<Option<Upstream>>,
}

impl<'t> Scoped<'t> {
    /// Returns an engine over `inner` that adds `headers` to every attempt.
    pub(crate) fn new(inner: &'t ReqwestTransport, headers: HeaderMap) -> Self {
        Self {
            inner,
            headers,
            answered: Mutex::new(None),
        }
    }

    /// Returns the last answer an attempt read, when one was read.
    pub(crate) fn answered(&self) -> Option<Upstream> {
        // NOTE: no specification governs this: our own design; a poisoned slot
        // still holds the last answer read, so it is taken as it stands.
        self.answered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl Transport for Scoped<'_> {
    async fn send(
        &self,
        mut request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, TransportError> {
        for (name, value) in &self.headers {
            request.headers_mut().append(name.clone(), value.clone());
        }
        *request.uri_mut() = with_literal_colons(request.uri())?;
        let response = self.inner.send(request).await?;
        let upstream = Upstream::new(
            response.status(),
            response.headers().clone(),
            String::from_utf8_lossy(response.body()).into_owned(),
        );
        *self.answered.lock().unwrap_or_else(PoisonError::into_inner) = Some(upstream);
        Ok(response)
    }
}

/// Returns `uri` with every `%3A` of its path written as the `:` it encodes.
///
/// A colon is a `pchar` (RFC 3986 §3.3), and a URI that percent-encodes a
/// reserved character is not equivalent to one that does not (§2.2), so an
/// `OBJECT_VERSION_ID` travels in a path segment as the
/// `object_id::creating_system_id::version_tree_id` ITS-REST 1.1.0 spells
/// (`ehr-codegen.openapi.yaml`, `components.parameters.uid_based_id`). The
/// generated client encodes every non-alphanumeric byte of a segment, and an
/// encoded `%` travels as `%25`, so a `%3A` in its output is always a colon.
///
/// # Errors
/// Returns [`TransportError::Send`] when the rewritten path is no legal URI
/// path, which a path of the generated client cannot be.
// TODO(#293): drop the rewrite once the generated client leaves `:` literal
// in a path segment (openehr-its sibling request).
fn with_literal_colons(uri: &http::Uri) -> Result<http::Uri, TransportError> {
    let Some(path_and_query) = uri.path_and_query() else {
        return Ok(uri.clone());
    };
    if !path_and_query.path().contains("%3A") {
        return Ok(uri.clone());
    }
    let mut rewritten = path_and_query.path().replace("%3A", ":");
    if let Some(query) = path_and_query.query() {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(http::uri::PathAndQuery::try_from(rewritten).map_err(
        |source| TransportError::Send {
            source: Box::new(source),
        },
    )?);
    http::Uri::from_parts(parts).map_err(|source| TransportError::Send {
        source: Box::new(source),
    })
}

#[cfg(test)]
mod tests {
    use super::with_literal_colons;

    #[test]
    fn a_version_id_segment_keeps_its_colons() {
        let uri: http::Uri =
            "http://cdr.invalid/v1/ehr/e1/composition/8849182c%3A%3Asystem%3A%3A1?version_at_time=2026-09-12T10%3A00%3A00"
                .parse()
                .expect("a legal URI");
        assert_eq!(
            "http://cdr.invalid/v1/ehr/e1/composition/8849182c::system::1?version_at_time=2026-09-12T10%3A00%3A00",
            with_literal_colons(&uri)
                .expect("the path rewrites")
                .to_string()
        );
    }

    #[test]
    fn an_encoded_percent_is_never_read_as_a_colon() {
        let uri: http::Uri = "http://cdr.invalid/v1/ehr/a%253Ab"
            .parse()
            .expect("a legal URI");
        assert_eq!(
            "http://cdr.invalid/v1/ehr/a%253Ab",
            with_literal_colons(&uri)
                .expect("the path rewrites")
                .to_string()
        );
    }

    #[test]
    fn a_path_with_a_space_keeps_its_encoding() {
        let uri: http::Uri = "http://cdr.invalid/v1/definition/template/adl1.4/Vital%20Signs"
            .parse()
            .expect("a legal URI");
        assert_eq!(
            uri.to_string(),
            with_literal_colons(&uri)
                .expect("the path rewrites")
                .to_string()
        );
    }
}
