// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Read, and the lookups the other interactions share with it.
//!
//! A read resolves the logical id through the identity map to one composition
//! and maps that composition back with the program the map recorded. An id the
//! map does not know is `404`; a composition the CDR reports deleted is `410`
//! (`docs/architecture.md` §4.6).

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::composition::CompositionOutcome;
use ferrobridge_openehr::composition::UidBasedId;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::ids::ObjectVersionId;
use ferrobridge_openehr::ids::VersionedObjectUid;
use ferrobridge_openehr::ids::entity_tag;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;
use openehr_mapping_core::composition::CanonicalComposition;

use crate::facade::Facade;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::render;
use crate::facade::handlers::write;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::record::CompositionBinding;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::programs::Loaded;
use crate::facade::reply;
use crate::facade::request::Inbound;
use crate::facade::status;

/// `GET [base]/{type}/{id}`.
pub(crate) async fn read(
    facade: &Facade,
    client: &Client,
    resource_type: &str,
    id: &str,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<axum::response::Response, Refusal> {
    crate::facade::handlers::guard(headers, uri, Body::Absent)?;
    crate::facade::handlers::supported(facade, resource_type)?;
    let internal = internal_id(id)?;
    let binding =
        binding(facade, resource_type, &internal)?.ok_or_else(|| unknown(resource_type, id))?;
    let program = program_of(facade, &binding)?;
    let ehr_id = EhrId::new(&binding.ehr_id).map_err(|error| stored_identifier(&error))?;
    let container = VersionedObjectUid::new(&binding.versioned_object_uid)
        .map_err(|error| stored_identifier(&error))?;
    let (version, composition) =
        fetch(facade, client, &ehr_id, &container, &binding.template_id).await?;
    let source = render::composition_url(&client.config().base_url, &ehr_id, &version)?;
    let rendered = render::render(
        program.program(),
        program.index(),
        &composition,
        &internal,
        &version,
        &source,
    )?;
    render::log_warnings(&rendered);
    Ok(reply::resource(
        StatusCode::OK,
        &rendered.body,
        &[reply::Header::entity_tag(version.version_tree_id())],
    ))
}

/// Reads one request body as a resource of `resource_type`.
pub(crate) fn parse(body: &[u8], resource_type: &str) -> Result<Inbound, Refusal> {
    Inbound::read(body, resource_type).map_err(|error| {
        let (status, code, location) = match &error {
            crate::facade::request::RequestError::Structure { location, .. } => (
                StatusCode::BAD_REQUEST,
                IssueType::Structure,
                Some(location.clone()),
            ),
            crate::facade::request::RequestError::NotJson { .. }
            | crate::facade::request::RequestError::NotAnObject => {
                (StatusCode::BAD_REQUEST, IssueType::Structure, None)
            }
            _ => (StatusCode::BAD_REQUEST, IssueType::Invalid, None),
        };
        let mut issue = Issue::error(code).diagnosing(render::chain(&error));
        if let Some(path) = location {
            issue = issue.at(path);
        }
        reply::refusal(status, issue)
    })
}

/// Returns the logical id `id` names, or the `404` a malformed one answers.
///
/// An id outside the R4 grammar can name no resource this server holds
/// (<https://hl7.org/fhir/R4/resource.html>), so it reads as unknown rather
/// than as a malformed request.
pub(crate) fn internal_id(id: &str) -> Result<FhirResourceId, Refusal> {
    FhirResourceId::new(id).map_err(|error| {
        reply::refusal(
            StatusCode::NOT_FOUND,
            Issue::error(IssueType::NotFound).diagnosing(render::chain(&error)),
        )
    })
}

/// Returns what the identity map holds for one logical id.
pub(crate) fn binding(
    facade: &Facade,
    resource_type: &str,
    id: &FhirResourceId,
) -> Result<Option<CompositionBinding>, Refusal> {
    facade
        .store()
        .binding_of(resource_type, id)
        .map_err(|error| write::store_refusal(&error))
}

/// Returns the program the identity map recorded for one binding.
pub(crate) fn program_of<'a>(
    facade: &'a Facade,
    binding: &CompositionBinding,
) -> Result<&'a Loaded, Refusal> {
    facade
        .programs()
        .by_context(&binding.context)
        .ok_or_else(|| {
            reply::refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception).diagnosing(format!(
                    "the identity map names the context {}, which this deployment did not load",
                    binding.context
                )),
            )
        })
}

/// Fetches the latest version of one composition.
pub(crate) async fn fetch(
    facade: &Facade,
    client: &Client,
    ehr_id: &EhrId,
    container: &VersionedObjectUid,
    template_id: &str,
) -> Result<(ObjectVersionId, CanonicalComposition), Refusal> {
    let answered = client
        .composition(
            ehr_id,
            &UidBasedId::VersionedObject(container.clone()),
            None,
        )
        .await
        .map_err(|error| write::refuse(&status::of_client_error(&error)))?;
    match answered {
        CompositionOutcome::Found {
            version_id,
            composition,
        } => {
            let version = version_id.ok_or_else(|| {
                reply::refusal(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Issue::error(IssueType::Exception)
                        .diagnosing("the CDR answered a composition with no ETag"),
                )
            })?;
            let text = openehr_its::json::to_canonical_json(composition.as_ref());
            let value = text.parse::<serde_json::Value>().map_err(|error| {
                reply::refusal(
                    StatusCode::BAD_GATEWAY,
                    Issue::error(IssueType::Exception).diagnosing(format!(
                        "the composition the CDR served could not be read back: {error}"
                    )),
                )
            })?;
            let generation = facade
                .programs()
                .loaded()
                .iter()
                .find(|entry| entry.index().template_id() == template_id)
                .map_or(openehr_mapping_core::template::Generation::Adl14, |entry| {
                    entry.index().generation()
                });
            Ok((
                version,
                CanonicalComposition::new(value, template_id, generation),
            ))
        }
        CompositionOutcome::Deleted => Err(write::refuse(&status::Answer::new(
            status::GONE,
            String::from("the CDR reports this composition deleted"),
        ))),
        CompositionOutcome::NotFound(upstream) => Err(write::refuse(&status::Answer::new(
            status::NOT_FOUND,
            status::diagnostics(&upstream),
        ))),
        other => Err(reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the openEHR client answered an outcome this version does not read ({other:?})"
            )),
        )),
    }
}

/// Returns the latest version of one composition, for a precondition check.
pub(crate) async fn latest_version(
    client: &Client,
    ehr_id: &EhrId,
    container: &VersionedObjectUid,
) -> Result<ObjectVersionId, Refusal> {
    let answered = client
        .composition(
            ehr_id,
            &UidBasedId::VersionedObject(container.clone()),
            None,
        )
        .await
        .map_err(|error| write::refuse(&status::of_client_error(&error)))?;
    match answered {
        CompositionOutcome::Found { version_id, .. } => version_id.ok_or_else(|| {
            reply::refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception)
                    .diagnosing("the CDR answered a composition with no ETag"),
            )
        }),
        CompositionOutcome::Deleted => Err(write::refuse(&status::Answer::new(
            status::GONE,
            String::from("the CDR reports this composition deleted"),
        ))),
        CompositionOutcome::NotFound(upstream) => Err(write::refuse(&status::Answer::new(
            status::NOT_FOUND,
            status::diagnostics(&upstream),
        ))),
        other => Err(reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the openEHR client answered an outcome this version does not read ({other:?})"
            )),
        )),
    }
}

/// Returns the openEHR version an `If-Match` names.
///
/// R4 sends the version id ("`If-Match: W/"2"`",
/// <https://hl7.org/fhir/R4/http.html#concurrency>), and ITS-REST takes the
/// whole `OBJECT_VERSION_ID`, so the two halves are joined here: a bare
/// version tree id is completed against the version container the map holds.
pub(crate) fn version_of_etag(
    value: &str,
    container: &VersionedObjectUid,
) -> Result<ObjectVersionId, Refusal> {
    let bare = entity_tag(value);
    if let Ok(full) = ObjectVersionId::new(bare) {
        return Ok(full);
    }
    // NOTE: a parse failure IS the answer here: a value that is no
    // OBJECT_VERSION_ID is the R4 spelling, which names the version tree id
    // alone (<https://hl7.org/fhir/R4/http.html#concurrency>).
    Err(reply::refusal(
        StatusCode::PRECONDITION_FAILED,
        Issue::error(IssueType::Conflict).diagnosing(format!(
            "If-Match carries `{bare}`, which names no version of {container}; send the ETag this server last answered with"
        )),
    ))
}

/// Returns the `404` an id the map does not know answers.
fn unknown(resource_type: &str, id: &str) -> Refusal {
    reply::refusal(
        StatusCode::NOT_FOUND,
        Issue::error(IssueType::NotFound)
            .diagnosing(format!("no mapping knows {resource_type}/{id}")),
    )
}

/// Returns the refusal a malformed stored identifier renders as.
fn stored_identifier(error: &ferrobridge_openehr::ids::IdError) -> Refusal {
    reply::refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the identity map holds an openEHR identifier this version cannot read: {error}"
        )),
    )
}
