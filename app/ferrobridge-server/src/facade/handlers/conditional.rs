// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `If-None-Exist`: the conditional create of FHIR R4.
//!
//! "If the search finds no matches, the server processes the create normally.
//! If it finds one match, the server returns `200 OK` … If it finds more than
//! one match, the server returns `412 Precondition Failed`"
//! (<https://hl7.org/fhir/R4/http.html#ccreate>).
//!
//! The search this milestone can answer runs against the identity map, so the
//! parameters are `_id` (a logical id this server assigned) and `identifier`
//! (a `Resource.identifier` a committed resource carried, which the ingest
//! records against its logical id), each admitting the comma-separated OR form
//! of FHIR search (<https://hl7.org/fhir/R4/search.html#combining>). Any other
//! parameter is refused rather than ignored, because a silently narrowed
//! search would turn a duplicate into a second composition.

use http::HeaderMap;
use http::StatusCode;
use std::collections::BTreeSet;

use crate::facade::Facade;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::read;
use crate::facade::handlers::write;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::record::IdentifierQuery;
use crate::facade::identity::record::SystemMatch;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;

/// The search parameters the conditional create answers.
const ANSWERED: [&str; 2] = ["_id", "identifier"];

/// Evaluates `If-None-Exist`, returning the answer when the create stops.
///
/// `None` means the create proceeds; `Some(response)` is the `200` of a single
/// match or the `412` of several, already rendered.
pub(crate) async fn if_none_exist(
    facade: &Facade,
    client: &crate::cdr::CdrClient,
    resource_type: &str,
    headers: &HeaderMap,
) -> Result<Option<axum::response::Response>, Refusal> {
    let Some(value) = headers.get("if-none-exist") else {
        return Ok(None);
    };
    let query = value.to_str().map_err(|_unreadable| {
        reply::refusal(
            StatusCode::BAD_REQUEST,
            Issue::error(IssueType::Invalid).diagnosing("the If-None-Exist header is not text"),
        )
    })?;
    let matches: Vec<FhirResourceId> = search(facade, resource_type, query)?.into_iter().collect();
    match matches.as_slice() {
        [] => Ok(None),
        [only] => read::read(
            facade,
            client,
            resource_type,
            only.as_str(),
            headers,
            &http::Uri::default(),
        )
        .await
        .map(Some),
        several => Err(reply::refusal(
            StatusCode::PRECONDITION_FAILED,
            Issue::error(IssueType::Duplicate).diagnosing(format!(
                "If-None-Exist matches {} resources, so this server cannot decide which one the create would duplicate",
                several.len()
            )),
        )),
    }
}

/// Returns the logical ids `query` matches, in order.
fn search(
    facade: &Facade,
    resource_type: &str,
    query: &str,
) -> Result<BTreeSet<FhirResourceId>, Refusal> {
    let mut found = BTreeSet::new();
    let mut seen_parameter = false;
    // NOTE: `If-None-Exist` carries "the search URL" query part
    // (<https://hl7.org/fhir/R4/http.html#ccreate>), so it is read percent-decoded.
    for (name, raw) in url::form_urlencoded::parse(query.trim_start_matches('?').as_bytes()) {
        if !ANSWERED.contains(&name.as_ref()) {
            return Err(unanswerable(&name));
        }
        seen_parameter = true;
        for value in raw.split(',').filter(|value| !value.is_empty()) {
            found.extend(resolve(facade, resource_type, &name, value)?);
        }
    }
    if !seen_parameter {
        return Err(reply::refusal(
            StatusCode::BAD_REQUEST,
            Issue::error(IssueType::Invalid)
                .diagnosing("If-None-Exist carries no search parameter"),
        ));
    }
    Ok(found)
}

/// Returns the logical ids one parameter value resolves to.
fn resolve(
    facade: &Facade,
    resource_type: &str,
    name: &str,
    value: &str,
) -> Result<Vec<FhirResourceId>, Refusal> {
    if name == "_id" {
        let Ok(id) = FhirResourceId::new(value) else {
            // NOTE: a value outside the R4 id grammar can name no resource
            // this server holds, so it legitimately matches nothing
            // (<https://hl7.org/fhir/R4/resource.html>).
            return Ok(Vec::new());
        };
        return Ok(read::binding(facade, resource_type, &id)?
            .map(|_held| id)
            .into_iter()
            .collect());
    }
    facade
        .store()
        .identified(resource_type, &token(value)?)
        .map_err(|error| write::store_refusal(&error))
}

/// Reads one `identifier` value as an R4 token.
///
/// The forms are `[code]` (any system), `|[code]` (no system) and
/// `[system]|[code]` (<https://hl7.org/fhir/R4/search.html#token>). The
/// `[system]|` form matches every code of a system, which the identity map
/// cannot enumerate by system, so it is refused.
fn token(value: &str) -> Result<IdentifierQuery, Refusal> {
    let Some((system, code)) = value.split_once('|') else {
        return Ok(IdentifierQuery::new(SystemMatch::Any, value));
    };
    if code.is_empty() {
        return Err(reply::refusal(
            StatusCode::BAD_REQUEST,
            Issue::error(IssueType::NotSupported).diagnosing(
                "If-None-Exist names identifier=[system]|, and this server answers an identifier search by its value",
            ),
        ));
    }
    let system = if system.is_empty() {
        SystemMatch::Absent
    } else {
        SystemMatch::Is(String::from(system))
    };
    Ok(IdentifierQuery::new(system, code))
}

/// Returns the refusal a parameter this milestone cannot answer renders as.
fn unanswerable(name: &str) -> Refusal {
    reply::refusal(
        StatusCode::BAD_REQUEST,
        Issue::error(IssueType::NotSupported).diagnosing(format!(
            "If-None-Exist names {name}, and this server answers a conditional create on {} only",
            ANSWERED.join(" and ")
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::ANSWERED;

    #[test]
    fn the_answered_parameters_are_the_two_the_identity_map_holds() {
        assert_eq!(["_id", "identifier"], ANSWERED);
    }
}
