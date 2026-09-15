// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! `$validate`: the dry run that commits nothing.
//!
//! "POST `[base]/{type}/$validate` runs the full inbound path … and answers
//! `200` with an `OperationOutcome` whose issues are `information` when the
//! resource would commit and `error` with the validator's message verbatim
//! when it would not" (`docs/architecture.md` §4.6). The operation-level
//! statuses mirror the create path, so the dry run cannot lie about the door
//! it models (<https://hl7.org/fhir/R4/resource-operation-validate.html>).
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
use crate::facade::engine;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::read;
use crate::facade::handlers::render;
use crate::facade::handlers::write;
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
    if let Err(refusal) = inbound.subject(&facade.settings().subject_namespace) {
        return Ok(reply::issues(
            StatusCode::OK,
            &[Issue::error(IssueType::Required)
                .diagnosing(render::chain(&refusal))
                .detailing(VALIDATOR)
                .at(format!("{resource_type}.subject"))],
            &[],
        ));
    }
    // NOTE: one instant serves every defaulted time of one run
    // (`docs/architecture.md` §12), and this one is discarded with the
    // composition it builds.
    let now = jiff::Timestamp::now().to_string();
    let built = match engine::inbound(program.program(), program.index(), inbound.document(), &now)
    {
        Ok(outcome) => outcome,
        Err(error) => return Ok(invalid(&render::chain(&error))),
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
    ];
    for warning in built.warnings() {
        issues.push(Issue::warning(IssueType::Informational).diagnosing(warning.to_string()));
    }
    Ok(reply::issues(StatusCode::OK, &issues, &[]))
}

/// Returns the `200` that reports a resource which would not commit.
///
/// The validator's message travels verbatim, which is what makes the dry run
/// worth running (`docs/architecture.md` §12).
fn invalid(message: &str) -> axum::response::Response {
    reply::issues(
        StatusCode::OK,
        &[Issue::error(IssueType::Invalid)
            .diagnosing(String::from(message))
            .detailing(VALIDATOR)],
        &[],
    )
}
