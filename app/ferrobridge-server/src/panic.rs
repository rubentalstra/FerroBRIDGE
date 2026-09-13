// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! A handler panic becomes a `500` with a JSON body, never a dropped
//! connection.
//!
//! The release profile pins `panic = "unwind"`, which is what lets
//! `std::panic::catch_unwind` turn an unwound handler into a response
//! (<https://doc.rust-lang.org/cargo/reference/profiles.html#panic>). The
//! panic message reaches the log and never the body. No specification governs
//! the body shape: our own design.

use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use std::any::Any;

/// The marker a caught panic leaves on its response.
///
/// The panic handler sees the payload and no request, so it marks the response
/// and [`render`] fills the request id in outside the layer that sets it.
#[derive(Debug, Clone)]
struct Panicked(String);

/// Renders a panicked handler as a `500` carrying its message as a marker.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the `CatchPanicLayer` handler contract hands the payload over by value"
)]
#[must_use]
pub fn caught(payload: Box<dyn Any + Send + 'static>) -> Response {
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| String::from("an unknown panic payload"));
    let mut response = body(StatusCode::INTERNAL_SERVER_ERROR, "internal", "").into_response();
    response.extensions_mut().insert(Panicked(message));
    response
}

/// Fills the request id into a panic response and logs the panic.
///
/// It runs outside the layer that propagates the request id, so the header is
/// already on the response and the body, the log line and the client's own
/// trace name the same request.
pub async fn render(response: Response) -> Response {
    let Some(panicked) = response.extensions().get::<Panicked>().cloned() else {
        return response;
    };
    let request_id = crate::request_id::of(response.headers())
        .unwrap_or_default()
        .to_owned();
    tracing::error!(
        request_id = request_id.as_str(),
        panic = panicked.0.as_str(),
        "the request handler panicked"
    );
    let mut rendered = body(response.status(), "internal", &request_id).into_response();
    // The request id header the propagate layer set is on the old response;
    // carry it so the client sees the same value the body names.
    *rendered.headers_mut() = response.headers().clone();
    rendered.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    rendered
}

/// Returns the JSON body this server answers an internal failure with.
fn body(status: StatusCode, error: &str, request_id: &str) -> (StatusCode, String) {
    let document = serde_json::json!({ "error": error, "request_id": request_id });
    (status, document.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Panicked, caught};
    use http::StatusCode;

    #[test]
    fn a_caught_panic_is_a_500_carrying_its_message_as_a_marker() {
        let response = caught(Box::new("the handler gave up"));
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, response.status());
        assert_eq!(
            Some("the handler gave up"),
            response
                .extensions()
                .get::<Panicked>()
                .map(|marked| marked.0.as_str())
        );
    }

    #[test]
    fn an_owned_message_and_an_unknown_payload_both_reach_the_marker() {
        let owned = caught(Box::new(String::from("owned message")));
        assert_eq!(
            Some("owned message"),
            owned.extensions().get::<Panicked>().map(|m| m.0.as_str())
        );
        let unknown = caught(Box::new(7_u8));
        assert_eq!(
            Some("an unknown panic payload"),
            unknown.extensions().get::<Panicked>().map(|m| m.0.as_str())
        );
    }
}
