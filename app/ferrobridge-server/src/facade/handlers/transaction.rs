// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! `POST [base]`: a `transaction` Bundle, all or nothing.
//!
//! "A transaction Bundle that cannot be mapped in full is refused with one
//! `OperationOutcome` naming every failing entry and nothing is committed"
//! (`docs/architecture.md` §4.6). So every entry is mapped first, and only a
//! Bundle that mapped in full reaches the CDR, as one CONTRIBUTION, which is
//! what makes the commit itself atomic (`ehr-codegen.openapi.yaml`,
//! `contribution_create`).
//!
//! `batch` has no implementation in this milestone and the
//! `CapabilityStatement` does not declare it, so a `batch` Bundle is refused
//! with `not-supported` (<https://hl7.org/fhir/R4/http.html#transaction>).

use ferrobridge_openehr::contribution::CreateContributionOutcome;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::prefer::Prefer;
use fhir_types::codec::Value;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::generated::ehr::Versionable;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::facade::Facade;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::read;
use crate::facade::handlers::render;
use crate::facade::handlers::write;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;
use crate::facade::request::Inbound;
use crate::facade::status;

/// The `Bundle.type` this route commits.
const TRANSACTION: &str = "transaction";

/// The `Bundle.type` this milestone refuses.
const BATCH: &str = "batch";

/// One entry that mapped, ready to commit.
#[derive(Debug)]
struct Mapped {
    /// The `fullUrl` the Bundle gave the entry, for a refusal that names it.
    full_url: String,
    /// The composition the entry produced.
    composition: Composition,
}

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
    let entries = bundle
        .document()
        .get("entry")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .to_vec();
    let (mapped, failures, subject) = map_entries(facade, &entries);
    if !failures.is_empty() {
        return Err(reply::refusals(
            StatusCode::UNPROCESSABLE_ENTITY,
            &failures,
            &[],
        ));
    }
    let Some(subject) = subject else {
        return Err(reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Required)
                .diagnosing("the Bundle carries no entry this server maps"),
        ));
    };
    let ehr_id = ehr::resolve(
        facade.client(),
        facade.store(),
        &subject,
        facade.settings().ehr_policy,
    )
    .await
    .map_err(|error| {
        reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Processing).diagnosing(render::chain(&error)),
        )
    })?;
    commit_all(facade, &ehr_id, &mapped).await
}

/// Maps every entry, collecting the refusals rather than stopping at the first.
///
/// A transaction is refused as a whole, so the answer names every failing
/// entry by its `fullUrl` and not only the one that failed first.
fn map_entries(
    facade: &Facade,
    entries: &[Value],
) -> (
    Vec<Mapped>,
    Vec<Issue>,
    Option<crate::facade::identity::PersonId>,
) {
    let mut mapped = Vec::new();
    let mut failures = Vec::new();
    let mut subject = None;
    for (index, entry) in entries.iter().enumerate() {
        let full_url = entry
            .get("fullUrl")
            .and_then(Value::as_str)
            .map_or_else(|| format!("Bundle.entry[{index}]"), str::to_owned);
        let Some(resource) = entry.get("resource") else {
            failures.push(
                Issue::error(IssueType::Required)
                    .diagnosing("the entry carries no resource")
                    .at(full_url),
            );
            continue;
        };
        match map_one(facade, resource, &full_url) {
            Ok((one, person)) => {
                match subject {
                    None => subject = Some(person),
                    Some(ref held) if held == &person => {}
                    Some(_) => {
                        failures.push(
                            Issue::error(IssueType::Processing)
                                .diagnosing(
                                    "the Bundle references more than one subject, and one Bundle maps to one composition",
                                )
                                .at(one.full_url.clone()),
                        );
                    }
                }
                mapped.push(one);
            }
            Err(issue) => failures.push(issue),
        }
    }
    (mapped, failures, subject)
}

/// Maps one Bundle entry, naming it by its `fullUrl` on refusal.
fn map_one(
    facade: &Facade,
    resource: &Value,
    full_url: &str,
) -> Result<(Mapped, crate::facade::identity::PersonId), Issue> {
    let inbound = Inbound::of(resource.clone(), "").map_err(|error| {
        Issue::error(IssueType::Structure)
            .diagnosing(render::chain(&error))
            .at(String::from(full_url))
    })?;
    if !facade.programs().supports(inbound.resource_type()) {
        return Err(Issue::error(IssueType::NotSupported)
            .diagnosing(format!(
                "no loaded mapping answers for {}",
                inbound.resource_type()
            ))
            .at(String::from(full_url)));
    }
    let program = facade
        .programs()
        .select(inbound.profiles(), None)
        .map_err(|error| {
            Issue::error(IssueType::NotSupported)
                .diagnosing(error.to_string())
                .at(String::from(full_url))
        })?;
    let subject = inbound
        .subject(&facade.settings().subject_namespace)
        .map_err(|error| {
            Issue::error(IssueType::Required)
                .diagnosing(render::chain(&error))
                .at(String::from(full_url))
        })?;
    // NOTE: one instant serves every defaulted time of one ingest
    // (`docs/architecture.md` §12), so each entry of a Bundle reads the clock
    // once and the whole Bundle commits with those readings.
    let now = jiff::Timestamp::now().to_string();
    let built = crate::facade::engine::inbound(
        program.program(),
        program.index(),
        inbound.document(),
        &now,
    )
    .map_err(|error| {
        Issue::error(IssueType::Processing)
            .diagnosing(render::chain(&error))
            .at(String::from(full_url))
    })?;
    let composition = built.into_value();
    let text = serde_json::to_string(composition.value()).map_err(|error| {
        Issue::error(IssueType::Exception)
            .diagnosing(format!(
                "the built composition could not be written: {error}"
            ))
            .at(String::from(full_url))
    })?;
    let rm = openehr_its::json::from_canonical_json::<Composition>(&text).map_err(|error| {
        Issue::error(IssueType::Processing)
            .diagnosing(format!(
                "the built composition is no valid openEHR COMPOSITION: {error}"
            ))
            .at(String::from(full_url))
    })?;
    Ok((
        Mapped {
            full_url: String::from(full_url),
            composition: rm,
        },
        subject,
    ))
}

/// Commits every mapped entry as one CONTRIBUTION.
async fn commit_all(
    facade: &Facade,
    ehr_id: &EhrId,
    mapped: &[Mapped],
) -> Result<axum::response::Response, Refusal> {
    let system_id = &facade.settings().system_id;
    let contribution = NewContribution {
        uid: None,
        versions: mapped
            .iter()
            .map(|entry| UpdateVersion {
                preceding_version_uid: None,
                signature: None,
                lifecycle_state: commit::lifecycle(),
                attestations: None,
                data: Versionable::Composition(entry.composition.clone()),
                commit_audit: commit::audit(commit::Change::Creation, system_id),
            })
            .collect(),
        audit: commit::audit(commit::Change::Creation, system_id),
    };
    let answered = facade
        .client()
        .create_contribution(ehr_id, &contribution, Prefer::Minimal)
        .await
        .map_err(|error| write::refuse(&status::of_client_error(&error)))?;
    match answered {
        CreateContributionOutcome::Created {
            contribution_uid, ..
        } => Ok(reply::issues(
            StatusCode::OK,
            &[
                Issue::information(IssueType::Informational).diagnosing(format!(
                    "{} entries committed as contribution {contribution_uid}",
                    mapped.len()
                )),
            ],
            &[],
        )),
        CreateContributionOutcome::BadRequest(upstream) => {
            Err(refuse_all(mapped, &status::BAD_REQUEST, &upstream))
        }
        CreateContributionOutcome::UnknownEhr(upstream) => {
            Err(refuse_all(mapped, &status::NOT_FOUND, &upstream))
        }
        CreateContributionOutcome::Conflict(upstream) => {
            Err(refuse_all(mapped, &status::PRECONDITION_FAILED, &upstream))
        }
        other => Err(reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the openEHR client answered an outcome this version does not read ({other:?})"
            )),
        )),
    }
}

/// Returns the refusal a rejected contribution renders as.
///
/// Nothing was committed, so the answer names every entry the Bundle carried:
/// a caller cannot tell from a partial list which entries still stand.
fn refuse_all(
    mapped: &[Mapped],
    row: &status::Row,
    upstream: &ferrobridge_openehr::error::UpstreamError,
) -> Refusal {
    let detail = status::diagnostics(upstream);
    let mut issues = vec![
        Issue::error(row.issue())
            .diagnosing(detail.clone())
            .detailing("the CDR refused the contribution, so no entry of this Bundle is stored"),
    ];
    for entry in mapped {
        issues.push(
            Issue::error(row.issue())
                .diagnosing(detail.clone())
                .at(entry.full_url.clone()),
        );
    }
    reply::refusals(row.status(), &issues, &[])
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
