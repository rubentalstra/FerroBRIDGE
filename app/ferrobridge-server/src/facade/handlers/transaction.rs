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
//! entry no program maps refuses the Bundle.
//!
//! `batch` has no implementation in this milestone and the
//! `CapabilityStatement` does not declare it, so a `batch` Bundle is refused
//! with `not-supported` (<https://hl7.org/fhir/R4/http.html#transaction>).

use fhir_types::codec::Value;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;

use crate::facade::Facade;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::read;
use crate::facade::ingest::Provenance;
use crate::facade::ingest::UnmappedEntries;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;

/// The `Bundle.type` this route commits.
const TRANSACTION: &str = "transaction";

/// The `Bundle.type` this milestone refuses.
const BATCH: &str = "batch";

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
    Ok(reply::issues(
        StatusCode::OK,
        &[
            Issue::information(IssueType::Informational).diagnosing(format!(
                "{} entries committed as contribution {}",
                ingested.committed().count(),
                ingested.contribution()
            )),
        ],
        &[],
    ))
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
