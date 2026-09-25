// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The interactions: metadata, create, read, update, transaction, `$validate`.
//!
//! Each handler is the same shape. It checks the media types and the query
//! parameters, resolves the program the request runs, calls the engine and the
//! CDR, and answers either a FHIR resource or an `OperationOutcome`
//! (`docs/architecture.md` §4.6). Every CDR answer reaches the wire through
//! the one table of [`crate::facade::status`], so a status is never invented
//! at a call site. Create, update and transaction write through
//! [`crate::facade::ingest`], so the handlers keep only the HTTP half.

mod conditional;
mod read;
mod render;
mod transaction;
mod validate;
mod write;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::Path;
use axum::extract::State;
use axum::response::Response;
use fhir_types::codec::Json;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;

use crate::facade::Facade;
use crate::facade::capability;
use crate::facade::media;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;

/// The refusal a handler short-circuits with, already rendered.
pub(crate) use crate::facade::reply::Refusal;

/// `GET [base]/metadata`: the `CapabilityStatement` of the loaded programs.
///
/// `operations` says whether the router serves the FHIRconnect operations
/// beside the facade, so the statement declares them only then.
#[must_use]
pub fn metadata_route(
    facade: &Facade,
    headers: &HeaderMap,
    uri: &Uri,
    operations: capability::Operations,
) -> Response {
    metadata(facade, headers, uri, operations).unwrap_or_else(Refusal::into_response)
}

/// `POST [base]/{type}`: create, conditional and idempotent by source
/// identity.
pub async fn create_route(
    State(facade): State<Arc<Facade>>,
    Path(resource_type): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    write::create(&facade, &resource_type, &headers, &uri, &body)
        .await
        .unwrap_or_else(Refusal::into_response)
}

/// `GET [base]/{type}/{id}`: read one mapped resource back out of the CDR.
pub async fn read_route(
    State(facade): State<Arc<Facade>>,
    Path((resource_type, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    read::read(
        &facade,
        &facade.client_for(&headers),
        &resource_type,
        &id,
        &headers,
        &uri,
    )
    .await
    .unwrap_or_else(Refusal::into_response)
}

/// `PUT [base]/{type}/{id}`: update a resource the identity map knows.
pub async fn update_route(
    State(facade): State<Arc<Facade>>,
    Path((resource_type, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    write::update(&facade, &resource_type, &id, &headers, &uri, &body)
        .await
        .unwrap_or_else(Refusal::into_response)
}

/// `POST [base]/{type}/$validate`: the dry run that commits nothing.
pub async fn validate_route(
    State(facade): State<Arc<Facade>>,
    Path(resource_type): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    validate::validate(&facade, &resource_type, &headers, &uri, &body)
        .unwrap_or_else(Refusal::into_response)
}

/// `POST [base]`: a `transaction` Bundle, all or nothing.
pub async fn transaction_route(
    State(facade): State<Arc<Facade>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    transaction::transaction(&facade, &headers, &uri, &body)
        .await
        .unwrap_or_else(Refusal::into_response)
}

/// Answers the `CapabilityStatement`.
fn metadata(
    facade: &Facade,
    headers: &HeaderMap,
    uri: &Uri,
    operations: capability::Operations,
) -> Result<Response, Refusal> {
    guard(headers, uri, Body::Absent)?;
    let statement =
        capability::statement(facade.programs(), &facade.settings().base_url, operations);
    let encoded = statement.to_json().map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the CapabilityStatement could not be encoded: {error}"
            )),
        )
    })?;
    Ok(reply::resource(StatusCode::OK, &encoded, &[]))
}

/// Whether the request carries a body whose media type is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Body {
    /// The interaction takes a body.
    Present,
    /// The interaction takes none.
    Absent,
}

/// Checks the media types and the query parameters of one request.
///
/// `Accept` that names nothing this server produces is `406`, a body media
/// type it does not read is `415`
/// (<https://hl7.org/fhir/R4/http.html#mime-type>), and a `_count` that is not
/// a page size is `400 invalid` rather than ignored
/// (<https://hl7.org/fhir/R4/search.html#count>).
pub(crate) fn guard(headers: &HeaderMap, uri: &Uri, body: Body) -> Result<(), Refusal> {
    if let Err(refusal) = media::check_accept(headers) {
        return Err(reply::refusal(
            StatusCode::NOT_ACCEPTABLE,
            Issue::error(IssueType::NotSupported).diagnosing(refusal.to_string()),
        ));
    }
    if body == Body::Present
        && let Err(refusal) = media::check_content_type(headers)
    {
        return Err(reply::refusal(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Issue::error(IssueType::NotSupported).diagnosing(refusal.to_string()),
        ));
    }
    if let Err(refusal) = media::count(uri.query()) {
        return Err(reply::refusal(
            StatusCode::BAD_REQUEST,
            Issue::error(IssueType::Invalid)
                .diagnosing(refusal.to_string())
                .at("_count"),
        ));
    }
    Ok(())
}

/// Checks that some loaded program maps `resource_type`.
///
/// "A type with no loaded context is `not-supported`"
/// (`docs/architecture.md` §4.6), and it is absent from the
/// `CapabilityStatement`, so the wire answer is a `404` whose issue code says
/// why.
pub(crate) fn supported(facade: &Facade, resource_type: &str) -> Result<(), Refusal> {
    if facade.programs().supports(resource_type) {
        return Ok(());
    }
    Err(reply::refusal(
        StatusCode::NOT_FOUND,
        Issue::error(IssueType::NotSupported).diagnosing(format!(
            "no loaded mapping answers for {resource_type}; GET {}/metadata names the types this server supports",
            crate::facade::BASE_PATH
        )),
    ))
}
