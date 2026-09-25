// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Create and update: the two inbound interactions.
//!
//! Create is conditional and idempotent by source identity (no specification
//! governs this: our own design, [`Ingest::recognise`] states the rule). A
//! resource whose `id` and `meta.versionId` the identity map already consumed
//! is a replay: the answer is `200 OK` naming the composition the first
//! delivery produced, and nothing is committed, as a re-sent transaction is
//! answered. A resource whose `id` the map consumed at another
//! `meta.versionId` commits a later version of that composition. A resource
//! with no `meta.versionId` and a known `id` is a replay when its mapping
//! matches the composition as it stands, and a later version otherwise. A
//! create claims its source before it reads the map and holds the claim until
//! it answers, so a second create of the same `id` that arrives meanwhile is a
//! `409` and commits nothing. Update needs the map to know the id, because
//! this milestone declares `updateCreate: false`, and it checks the CDR's
//! current `ETag` when the client sends no `If-Match`.
//!
//! The handlers own the HTTP half: the media types, the `templateId` pin,
//! `If-None-Exist`, `If-Match` and `Prefer`. The map, the EHR resolution, the
//! commit and the identity record are [`crate::facade::ingest`]'s.

use crate::cdr::CdrClient;
use crate::cdr::ids::EhrId;
use fhirconnect::resolve::program::TemplateId;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;
use http::header;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

use crate::facade::Facade;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::conditional;
use crate::facade::handlers::read;
use crate::facade::handlers::render;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::ingest;
use crate::facade::ingest::Ingest;
use crate::facade::ingest::Provenance;
use crate::facade::ingest::Recognised;
use crate::facade::ingest::Refused;
use crate::facade::ingest::Written;
use crate::facade::media;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::programs::Loaded;
use crate::facade::reply;
use crate::facade::request::Inbound;
use crate::facade::status;

/// `POST [base]/{type}`.
pub(crate) async fn create(
    facade: &Facade,
    resource_type: &str,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<axum::response::Response, Refusal> {
    crate::facade::handlers::guard(headers, uri, Body::Present)?;
    crate::facade::handlers::supported(facade, resource_type)?;
    let inbound = read::parse(body, resource_type)?;
    let program = select_program(facade, &inbound, headers)?;
    let ingest = facade.ingest(facade.client_for(headers));
    let claim = ingest.claim_resource(&inbound)?;

    match ingest
        .recognise(&claim, &inbound, program, &Provenance::EachResource)
        .await?
    {
        Recognised::Replayed(written) => {
            return answer(facade, program, &written, headers, StatusCode::OK);
        }
        Recognised::Revised(known) => {
            return resend(facade, &ingest, program, &inbound, &known, headers).await;
        }
        Recognised::Unknown => {}
    }
    if let Some(answer) =
        conditional::if_none_exist(facade, ingest.client(), resource_type, headers).await?
    {
        return Ok(answer);
    }
    let written = ingest
        .ingest_resource(&claim, &inbound, program, &Provenance::EachResource)
        .await?;
    let status = match written.delivery {
        ingest::Delivery::Committed => StatusCode::CREATED,
        ingest::Delivery::Reconciled | ingest::Delivery::Replayed => StatusCode::OK,
    };
    answer(facade, program, &written, headers, status)
}

/// `PUT [base]/{type}/{id}`.
pub(crate) async fn update(
    facade: &Facade,
    resource_type: &str,
    id: &str,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
) -> Result<axum::response::Response, Refusal> {
    crate::facade::handlers::guard(headers, uri, Body::Present)?;
    crate::facade::handlers::supported(facade, resource_type)?;
    let internal = read::internal_id(id)?;
    let binding = read::binding(facade, resource_type, &internal)?.ok_or_else(|| {
        // NOTE: `updateCreate` is false in the CapabilityStatement, so a PUT
        // to an unknown id is a 404 rather than a create
        // (<https://hl7.org/fhir/R4/http.html#update>).
        reply::refusal(
            StatusCode::NOT_FOUND,
            Issue::error(IssueType::NotFound).diagnosing(format!(
                "no mapping knows {resource_type}/{id}; this server does not create a resource on update"
            )),
        )
    })?;
    let inbound = read::parse(body, resource_type)?;
    let program = select_program(facade, &inbound, headers)?;
    let known = ConsumedSource {
        ehr_id: binding.ehr_id.clone(),
        versioned_object_uid: binding.versioned_object_uid.clone(),
        internal_id: String::from(internal.as_str()),
        context: binding.context.clone(),
    };
    let ingest = facade.ingest(facade.client_for(headers));
    resend(facade, &ingest, program, &inbound, &known, headers).await
}

/// Runs the update path of a resource the identity map already placed.
async fn resend(
    facade: &Facade,
    ingest: &Ingest<'_>,
    program: &Loaded,
    inbound: &Inbound,
    known: &ConsumedSource,
    headers: &HeaderMap,
) -> Result<axum::response::Response, Refusal> {
    let ehr_id = EhrId::new(&known.ehr_id).map_err(|error| store_identifier(&error))?;
    let container =
        HierObjectId::new(&known.versioned_object_uid).map_err(|error| store_identifier(&error))?;
    let preceding = precondition(ingest.client(), headers, &ehr_id, &container).await?;
    let written = ingest
        .revise_resource(
            inbound,
            program,
            &ehr_id,
            &container,
            &preceding,
            &Provenance::EachResource,
        )
        .await?;
    answer(facade, program, &written, headers, StatusCode::OK)
}

/// Answers one committed write, honouring `Prefer`.
fn answer(
    facade: &Facade,
    program: &Loaded,
    written: &Written,
    headers: &HeaderMap,
    status: StatusCode,
) -> Result<axum::response::Response, Refusal> {
    let source = render::composition_url(
        facade.client().base_url(),
        &written.ehr_id,
        &written.version,
    )?;
    let location = format!(
        "{}/{}/{}/_history/{}",
        facade.settings().base_url.trim_end_matches('/'),
        program.program().resource().as_str(),
        written.id,
        written.version.version_tree_id().value()
    );
    let response_headers = [
        reply::Header::entity_tag(written.version.version_tree_id().value()),
        reply::Header::location(&location),
    ];
    match media::Prefer::of(headers) {
        media::Prefer::Minimal => Ok(reply::minimal(status, &response_headers)),
        media::Prefer::Outcome => Ok(reply::issues(
            status,
            &[
                Issue::information(IssueType::Informational).diagnosing(format!(
                    "the resource is committed as version {}",
                    written.version.value()
                )),
            ],
            &response_headers,
        )),
        media::Prefer::Representation => {
            let rendered = render::render(
                program.program(),
                program.index(),
                &written.composition,
                &written.id,
                &written.version,
                &source,
            )?;
            render::log_warnings(&rendered);
            Ok(reply::resource(status, &rendered.body, &response_headers))
        }
    }
}

/// Returns the version a write must follow, from `If-Match` or from the CDR.
///
/// A missing `If-Match` checks the CDR's current `ETag` (no specification
/// governs this: our own design): the facade reads the latest version and uses
/// it, so a concurrent writer still produces the `412` the CDR answers.
async fn precondition(
    client: &CdrClient,
    headers: &HeaderMap,
    ehr_id: &EhrId,
    container: &HierObjectId,
) -> Result<ObjectVersionId, Refusal> {
    if let Some(value) = headers.get(header::IF_MATCH) {
        let text = value.to_str().map_err(|_unreadable| {
            reply::refusal(
                StatusCode::BAD_REQUEST,
                Issue::error(IssueType::Invalid).diagnosing("the If-Match header is not text"),
            )
        })?;
        return read::version_of_etag(text, container);
    }
    read::latest_version(client, ehr_id, container).await
}

/// Selects the program one inbound resource runs.
pub(crate) fn select_program<'a>(
    facade: &'a Facade,
    inbound: &Inbound,
    headers: &HeaderMap,
) -> Result<&'a Loaded, Refusal> {
    let pin = template_pin(headers);
    Ok(ingest::select(facade.programs(), inbound, pin.as_ref())?)
}

/// Returns the `templateId` pin a request carries, when it carries one.
///
/// The pin is the `templateId` parameter of the FHIRconnect operations, sent
/// here as an `X-FerroBRIDGE-Template-Id` header so it does not collide with a
/// FHIR search parameter (no specification governs the spelling: our own
/// design).
fn template_pin(headers: &HeaderMap) -> Option<TemplateId> {
    headers
        .get("x-ferrobridge-template-id")
        .and_then(|value| value.to_str().ok())
        .filter(|text| !text.is_empty())
        .map(TemplateId::new)
}

/// Returns the refusal one status-table answer renders as.
pub(crate) fn refuse(answer: &status::Answer) -> Refusal {
    Refusal::from(Refused::of_answer(answer))
}

/// Returns the refusal an identity-store failure renders as.
pub(crate) fn store_refusal(error: &crate::facade::identity::store::StoreError) -> Refusal {
    Refusal::from(ingest::store_refusal(error))
}

/// Returns the refusal a malformed stored identifier renders as.
fn store_identifier(error: &impl std::fmt::Display) -> Refusal {
    reply::refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the identity map holds an openEHR identifier this version cannot read: {error}"
        )),
    )
}
