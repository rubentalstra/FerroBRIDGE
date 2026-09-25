// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `POST [base]`: a `transaction` Bundle, all or nothing.
//!
//! A transaction Bundle that cannot be mapped in full is refused with one
//! `OperationOutcome` naming every failing entry and nothing is committed
//! (the refusal shape is our own design). So every entry is mapped first, and only a
//! Bundle that mapped in full reaches the CDR, as one CONTRIBUTION, which is
//! what makes the commit itself atomic (`ehr-codegen.openapi.yaml`,
//! `contribution_create`). The mapping and the commit are
//! [`crate::facade::ingest`]'s, run with [`UnmappedEntries::Refuse`] so an
//! entry no program maps refuses the Bundle. A committed Bundle answers a
//! `transaction-response` Bundle naming each entry's resource id and version,
//! and a Bundle an earlier delivery already committed answers the same ids
//! without a second commit. An entry carrying a new `meta.versionId` of a
//! resource the identity map knows commits a later version of its
//! composition inside the same CONTRIBUTION.
//!
//! `batch` has no implementation in this milestone and the
//! `CapabilityStatement` does not declare it, so a `batch` Bundle is refused
//! with `not-supported` (<https://hl7.org/fhir/R4/http.html#transaction>).

use fhir_types::codec::Json as _;
use fhir_types::codec::Value;
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::bundle::BundleEntry;
use fhir_types::r4::bundle::BundleEntryResponse;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;

use crate::facade::Facade;
use crate::facade::commit::Change;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::read;
use crate::facade::ingest::Ingested;
use crate::facade::ingest::Provenance;
use crate::facade::ingest::UnmappedEntries;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;

/// The `Bundle.type` this route commits.
const TRANSACTION: &str = "transaction";

/// The `Bundle.type` this milestone refuses.
const BATCH: &str = "batch";

/// The `Bundle.type` a committed transaction answers with.
const RESPONSE: &str = "transaction-response";

/// `POST [base]`.
pub(crate) async fn transaction(
    facade: &Facade,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<axum::response::Response, Refusal> {
    crate::facade::handlers::guard(headers, uri, Body::Present)?;
    let bundle = read::parse(body, "Bundle")?;
    let kind = bundle
        .document()
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == BATCH {
        return Err(reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::NotSupported).diagnosing(
                "this server commits a transaction Bundle; batch is not among the interactions its CapabilityStatement declares",
            ),
        ));
    }
    if kind != TRANSACTION {
        return Err(reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::NotSupported)
                .diagnosing(format!(
                    "Bundle.type is `{kind}`; this route commits a transaction Bundle"
                ))
                .at("Bundle.type"),
        ));
    }
    let ingested = facade
        .ingest(facade.client_for(headers))
        .ingest_bundle(
            bundle.document(),
            UnmappedEntries::Refuse,
            &Provenance::EachResource,
        )
        .await?;
    let response = response_bundle(&facade.settings().base_url, &ingested);
    let body = response.to_json().map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the transaction-response Bundle could not be encoded: {error}"
            )),
        )
    })?;
    Ok(reply::resource(StatusCode::OK, &body, &[]))
}

/// Returns the `transaction-response` Bundle of one ingested transaction.
///
/// "the server SHALL return a Bundle with type set to … `transaction-response`
/// that contains one entry for each entry in the request, in the same order",
/// each with a `response` carrying the status, `location` and `ETag`
/// (<https://hl7.org/fhir/R4/http.html#transaction-response>). The location and
/// the tag are the ones a single create answers. An entry this delivery
/// committed as a first version answers `201 Created`; one it committed as a
/// later version of a known resource answers `200 OK`, as a single create of
/// that resource does, and so does one an earlier delivery consumed.
fn response_bundle(base_url: &str, ingested: &Ingested) -> Bundle {
    let base = base_url.trim_end_matches('/');
    Bundle {
        r#type: RESPONSE.into(),
        entry: ingested
            .committed()
            .map(|entry| BundleEntry {
                response: Some(BundleEntryResponse {
                    status: match entry.change {
                        Some(Change::Creation) => "201 Created",
                        Some(Change::Modification) | None => "200 OK",
                    }
                    .into(),
                    location: Some(
                        format!(
                            "{base}/{}/{}/_history/{}",
                            entry.resource_type,
                            entry.id,
                            entry.version.version_tree_id().value()
                        )
                        .into(),
                    ),
                    etag: Some(format!("W/\"{}\"", entry.version.version_tree_id().value()).into()),
                    ..BundleEntryResponse::default()
                }),
                ..BundleEntry::default()
            })
            .collect(),
        ..Bundle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{BATCH, TRANSACTION};

    #[test]
    fn the_two_bundle_types_this_route_distinguishes_are_the_r4_codes() {
        assert_eq!("transaction", TRANSACTION);
        assert_eq!("batch", BATCH);
    }
}
