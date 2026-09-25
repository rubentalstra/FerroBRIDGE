// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `$validate`: the dry run that commits nothing.
//!
//! `POST [base]/{type}/$validate` runs the full inbound path and answers `200`
//! with an `OperationOutcome` whose issues are `information`, naming the
//! template and where the composition would be written, when the resource
//! would commit, and `error` with the validator's message verbatim when it
//! would not. The operation-level statuses mirror the create path, so the dry
//! run cannot lie about the door it models
//! (<https://hl7.org/fhir/R4/resource-operation-validate.html>). No
//! specification governs the issue content: our own design.
//!
//! ITS-REST 1.1.0 documents no composition-validation route
//! (`ehr-codegen.openapi.yaml` defines `/ehr/{ehr_id}/composition` and
//! `/ehr/{ehr_id}/composition/{uid_based_id}` and nothing else), and a commit
//! the bridge never finalises would still write a version, so the validator
//! here is the bridge's own: the engine run, the template-index validation
//! that `build_composition` performs, and the strict RM read. The outcome says
//! so in `issue.details`.

use http::HeaderMap;
use http::StatusCode;
use http::Uri;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::facade::Facade;
use crate::facade::ehr::Policy;
use crate::facade::engine;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::read;
use crate::facade::handlers::write;
use crate::facade::identity::PersonId;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::reply;

/// What the outcome names as the validator that ran.
const VALIDATOR: &str = "the FerroBRIDGE mapping engine, the Web Template validation of the composition build, and the strict openEHR RM reader; openEHR ITS-REST 1.1.0 documents no composition-validation route, and this dry run commits nothing";

/// `POST [base]/{type}/$validate`.
pub(crate) fn validate(
    facade: &Facade,
    resource_type: &str,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<axum::response::Response, Refusal> {
    crate::facade::handlers::guard(headers, uri, Body::Present)?;
    crate::facade::handlers::supported(facade, resource_type)?;
    let inbound = read::parse(body, resource_type)?;
    let program = write::select_program(facade, &inbound, headers)?;
    let person = match inbound.subject(&facade.settings().subject_namespace) {
        Ok(person) => person,
        Err(refusal) => {
            return Ok(reply::issues(
                StatusCode::OK,
                &[Issue::error(IssueType::Required)
                    .diagnosing(crate::facade::outcome::chain(&refusal))
                    .detailing(VALIDATOR)
                    .at(format!("{resource_type}.subject"))],
                &[],
            ));
        }
    };
    // NOTE: no specification governs this: our own design, one instant serves
    // every defaulted time of one run, and this one is discarded with the
    // composition it builds.
    let now = jiff::Timestamp::now().to_string();
    let built = match engine::inbound(
        program.program(),
        program.index(),
        inbound.document(),
        &now,
        facade.settings(),
    ) {
        Ok(outcome) => outcome,
        Err(error) => return Ok(invalid(&crate::facade::outcome::chain(&error))),
    };
    let text = serde_json::to_string(built.value().value()).map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the built composition could not be written: {error}"
            )),
        )
    })?;
    if let Err(error) = openehr_its::json::from_canonical_json::<Composition>(&text) {
        return Ok(invalid(&error.to_string()));
    }
    let mut issues = vec![
        Issue::information(IssueType::Informational)
            .diagnosing(format!(
                "the resource maps to a composition of {} that this server would commit",
                built.value().template_id()
            ))
            .detailing(VALIDATOR),
        Issue::information(IssueType::Informational)
            .diagnosing(disposition(facade, &person)?)
            .detailing(VALIDATOR),
    ];
    for warning in built.warnings() {
        issues.push(Issue::warning(IssueType::Informational).diagnosing(warning.to_string()));
    }
    Ok(reply::issues(StatusCode::OK, &issues, &[]))
}

/// Returns where the composition would be written, as the create path would
/// resolve it.
///
/// The dry run reads the identity map, which is local, and never asks the
/// CDR, so an EHR the map does not know yet is reported by the policy that
/// would decide it rather than looked up. No specification governs this: our
/// own design.
fn disposition(facade: &Facade, person: &PersonId) -> Result<String, Refusal> {
    let known = facade.store().ehr_of(person).map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(crate::facade::outcome::chain(&error)),
        )
    })?;
    Ok(match (known, facade.settings().ehr_policy) {
        (Some(ehr_id), _) => format!(
            "the composition would be written into the EHR {} that the identity map holds for \
             the subject {person}",
            ehr_id.as_str()
        ),
        (None, Policy::Existing) => format!(
            "the identity map holds no EHR for the subject {person}; the create path looks it up \
             in the CDR and refuses when none exists, because the facade writes into an \
             existing EHR only"
        ),
        (None, Policy::CreateOnFirstWrite) => format!(
            "the identity map holds no EHR for the subject {person}; the create path looks it up \
             in the CDR and creates one when none exists"
        ),
    })
}

/// Returns the `200` that reports a resource which would not commit.
///
/// The validator's message travels verbatim, which is what makes the dry run
/// worth running.
fn invalid(message: &str) -> axum::response::Response {
    reply::issues(
        StatusCode::OK,
        &[Issue::error(IssueType::Invalid)
            .diagnosing(String::from(message))
            .detailing(VALIDATOR)],
        &[],
    )
}
