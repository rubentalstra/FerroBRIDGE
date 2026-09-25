// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The engine one call sends through: the configured `reqwest` engine, plus
//! the headers the generated parameters do not carry and a record of the last
//! answer.
//!
//! No specification governs this: our own design. The generated parameters
//! carry no `X-Request-Id` and a read's parameters no `Prefer`, so the engine
//! adds both; the generated outcomes carry only the response headers the
//! `OpenAPI` documents declare, so the engine keeps the whole answer for the
//! ones they leave out (the `ETag` of an `adl2` template, the `ETag` of a
//! committed contribution whose body does not decode).

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
