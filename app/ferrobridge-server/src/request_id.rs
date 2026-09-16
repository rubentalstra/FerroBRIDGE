// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The correlation identifier every request carries.
//!
//! A client value is echoed when it is short printable ASCII, so a header
//! cannot smuggle a line break or a control character into a log line; every
//! other request gets a minted version 4 UUID. No specification governs the
//! header: our own design, and the name every proxy already uses.

use axum::extract::Request;
use http::{HeaderName, HeaderValue};
use tower_http::request_id::{MakeRequestId, RequestId};

/// The header a client sends to name its request, and the server echoes.
pub const HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// The longest client value this server echoes.
pub const MAX_LENGTH: usize = 128;

/// Returns whether `value` may be echoed as a request id.
///
/// The rule is printable ASCII (`0x20` to `0x7e`) of at most [`MAX_LENGTH`]
/// characters and never empty, so the value is safe on one log line and in one
/// header.
#[must_use]
pub fn is_legal(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LENGTH
        && value.chars().all(|c| c.is_ascii_graphic() || c == ' ')
}

/// Mints a fresh request id for every request that needs one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mint;

impl MakeRequestId for Mint {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
            .ok()
            .map(RequestId::new)
    }
}

/// Removes an `x-request-id` this server will not echo.
///
/// It runs outside the layer that mints one, so an illegal client value leaves
/// no header behind and the minted id takes its place.
pub async fn strip_illegal(mut request: Request) -> Request {
    let legal = request
        .headers()
        .get(HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_legal);
    if !legal {
        request.headers_mut().remove(HEADER);
    }
    request
}

/// Returns the request id `headers` carries, when it carries a legal one.
#[must_use]
pub fn of(headers: &http::HeaderMap) -> Option<&str> {
    headers
        .get(HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_legal(value))
}

#[cfg(test)]
mod tests {
    use super::{MAX_LENGTH, is_legal};

    #[test]
    fn a_printable_ascii_value_of_bounded_length_is_echoed() {
        assert!(is_legal("corr-42"));
        assert!(is_legal("a b"), "a space is printable");
        assert!(is_legal(&"a".repeat(MAX_LENGTH)));
    }

    #[test]
    fn a_control_character_a_non_ascii_byte_an_empty_value_and_an_over_long_one_are_refused() {
        assert!(!is_legal("corr\n42"), "a line break would forge a log line");
        assert!(!is_legal("corr\r42"));
        assert!(!is_legal("corr\t42"));
        assert!(!is_legal("corr\u{7f}42"), "DEL is not printable");
        assert!(!is_legal("corr\u{e9}42"), "only ASCII is echoed");
        assert!(!is_legal(""));
        assert!(!is_legal(&"a".repeat(MAX_LENGTH + 1)));
    }
}
