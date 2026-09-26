// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Read and vread, and the lookups the other interactions share with them.
//!
//! A read resolves the logical id through the identity map to one composition
//! and maps that composition back with the program the map recorded. An id the
//! map does not know is `404`; a composition the CDR reports deleted is `410`
//! (`docs/architecture.md` §4.6). A vread does the same for the composition
//! version whose version tree id is the `[vid]` a write's `Location` named
//! (<https://hl7.org/fhir/R4/http.html#vread>).

use http::HeaderMap;
use http::StatusCode;
use http::Uri;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_base::v1_3::base_types::identification::version_tree_id::VersionTreeId;
use openehr_its::rest::generated::ehr::client::CompositionGetOutcome;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::CdrClient;
use crate::cdr::ids::EhrId;
use crate::cdr::ids::entity_tag;
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
    client: &CdrClient,
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
    let container = HierObjectId::new(&binding.versioned_object_uid)
        .map_err(|error| stored_identifier(&error))?;
    let (version, composition) =
        fetch(facade, client, &ehr_id, &container, &binding.template_id).await?;
    let source = render::composition_url(client.base_url(), &ehr_id, &version)?;
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
        &[reply::Header::entity_tag(version.version_tree_id().value())],
    ))
}

/// `GET [base]/{type}/{id}/_history/{vid}`.
///
/// The `[vid]` is the version tree id every write answers in its `Location`
/// and `ETag`, so the version it names joins the version container the map
/// binds, the creating system of the latest version, and `[vid]` (no
/// specification governs the inversion: our own design). A `[vid]` that is no
/// version tree id, or that the CDR does not hold, is `404`; a version the CDR
/// reports deleted is `410`.
pub(crate) async fn vread(
    facade: &Facade,
    client: &CdrClient,
    target: (&str, &str, &str),
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<axum::response::Response, Refusal> {
    let (resource_type, id, vid) = target;
    crate::facade::handlers::guard(headers, uri, Body::Absent)?;
    crate::facade::handlers::supported(facade, resource_type)?;
    let internal = internal_id(id)?;
    let binding =
        binding(facade, resource_type, &internal)?.ok_or_else(|| unknown(resource_type, id))?;
    let program = program_of(facade, &binding)?;
    let tree = VersionTreeId::new(vid).map_err(|error| {
        reply::refusal(
            StatusCode::NOT_FOUND,
            Issue::error(IssueType::NotFound).diagnosing(format!(
                "{resource_type}/{id} has no version {vid}: {}",
                crate::facade::outcome::chain(&error)
            )),
        )
    })?;
    let ehr_id = EhrId::new(&binding.ehr_id).map_err(|error| stored_identifier(&error))?;
    let container = HierObjectId::new(&binding.versioned_object_uid)
        .map_err(|error| stored_identifier(&error))?;
    let (newest, newest_body) = latest(client, &ehr_id, &container).await?;
    let (version, body) = if newest.version_tree_id().value() == tree.value() {
        (newest, newest_body)
    } else {
        let wanted = ObjectVersionId::new(format!(
            "{}::{}::{}",
            container.value(),
            newest.creating_system_id_str(),
            tree.value()
        ))
        .map_err(|error| stored_identifier(&error))?;
        let (named, body) = answered_version(
            client,
            &ehr_id,
            &UidBasedId::ObjectVersionId(wanted.clone()),
        )
        .await?;
        if let Some(named) = named
            && named != wanted
        {
            return Err(reply::refusal(
                StatusCode::BAD_GATEWAY,
                Issue::error(IssueType::Exception).diagnosing(format!(
                    "the CDR answered version {} where {} was asked for",
                    named.value(),
                    wanted.value()
                )),
            ));
        }
        (wanted, body)
    };
    let composition = canonical(facade, &body, &binding.template_id)?;
    let source = render::composition_url(client.base_url(), &ehr_id, &version)?;
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
        &[reply::Header::entity_tag(version.version_tree_id().value())],
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
        let mut issue = Issue::error(code).diagnosing(crate::facade::outcome::chain(&error));
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
            Issue::error(IssueType::NotFound).diagnosing(crate::facade::outcome::chain(&error)),
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
    client: &CdrClient,
    ehr_id: &EhrId,
    container: &HierObjectId,
    template_id: &str,
) -> Result<(ObjectVersionId, CanonicalComposition), Refusal> {
    let (version, composition) = latest(client, ehr_id, container).await?;
    Ok((version, canonical(facade, &composition, template_id)?))
}

/// Returns the canonical form of one composition the CDR served.
fn canonical(
    facade: &Facade,
    composition: &Composition,
    template_id: &str,
) -> Result<CanonicalComposition, Refusal> {
    let text = openehr_its::json::to_canonical_json(composition);
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
    Ok(CanonicalComposition::new(value, template_id, generation))
}

/// Returns the latest version of one composition, for a precondition check.
pub(crate) async fn latest_version(
    client: &CdrClient,
    ehr_id: &EhrId,
    container: &HierObjectId,
) -> Result<ObjectVersionId, Refusal> {
    latest(client, ehr_id, container)
        .await
        .map(|(version, _composition)| version)
}

/// Reads the latest version of the composition `container` holds, with the
/// version its `ETag` names.
async fn latest(
    client: &CdrClient,
    ehr_id: &EhrId,
    container: &HierObjectId,
) -> Result<(ObjectVersionId, Composition), Refusal> {
    let (version, body) =
        answered_version(client, ehr_id, &UidBasedId::HierObjectId(container.clone())).await?;
    let version = version.ok_or_else(|| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception)
                .diagnosing("the CDR answered a composition with no ETag"),
        )
    })?;
    Ok((version, body))
}

/// Reads the composition version `uid_based_id` names, with the version its
/// `ETag` names when the CDR sent one.
///
/// A `204` is the CDR reporting the version deleted, which is `410`
/// (`ehr-codegen.openapi.yaml`, `composition_get`).
async fn answered_version(
    client: &CdrClient,
    ehr_id: &EhrId,
    uid_based_id: &UidBasedId,
) -> Result<(Option<ObjectVersionId>, Composition), Refusal> {
    let answered = client
        .composition(ehr_id, uid_based_id, None)
        .await
        .map_err(|error| write::refuse(&status::of_client_error(&error)))?;
    match answered.outcome {
        CompositionGetOutcome::Ok { body, headers } => {
            let version =
                crate::cdr::optional_version_from_etag("composition_get", headers.etag.as_deref())
                    .map_err(|error| write::refuse(&status::of_client_error(&error)))?;
            Ok((version, body))
        }
        CompositionGetOutcome::NoContent => Err(write::refuse(&status::Answer::new(
            status::GONE,
            String::from("the CDR reports this composition deleted"),
        ))),
        CompositionGetOutcome::NotFound { body } => Err(write::refuse(&status::Answer::new(
            status::NOT_FOUND,
            status::diagnostics(StatusCode::NOT_FOUND, &body),
        ))),
    }
}

/// Returns the openEHR version an `If-Match` names, checked against the latest
/// version `container` holds.
///
/// R4 sends the `versionId` as a weak entity tag ("`If-Match: W/"2"`",
/// <https://hl7.org/fhir/R4/http.html#concurrency>), and ITS-REST takes the
/// whole `OBJECT_VERSION_ID`. A version tree id (`W/"N"`, `"N"` or `N`) that
/// names the latest version completes to it, as vread completes its `[vid]`;
/// one that names any other version is `412` carrying the current `ETag`. The
/// CDR's own `uid::system::N` of `container` passes through for the CDR to
/// check, and one of another container is `412`.
///
/// # Errors
///
/// A `400` for a value that names no version at all, a `412` for a stale or
/// foreign version, and the refusal the read of the latest version answers.
pub(crate) async fn version_of_etag(
    client: &CdrClient,
    ehr_id: &EhrId,
    container: &HierObjectId,
    value: &str,
) -> Result<ObjectVersionId, Refusal> {
    let bare = entity_tag(value);
    if let Ok(full) = ObjectVersionId::new(bare) {
        if full.object_id().value() != container.value() {
            return Err(write::refuse(&status::Answer::new(
                status::PRECONDITION_FAILED,
                format!(
                    "If-Match carries `{bare}`, a version of another composition than {}",
                    container.value()
                ),
            )));
        }
        return Ok(full);
    }
    // NOTE: a parse failure IS the answer here: a value that is no
    // OBJECT_VERSION_ID is the R4 spelling, which names the version tree id
    // alone (<https://hl7.org/fhir/R4/http.html#concurrency>).
    let tree = VersionTreeId::new(bare).map_err(|error| {
        reply::refusal(
            StatusCode::BAD_REQUEST,
            Issue::error(IssueType::Invalid).diagnosing(format!(
                "If-Match carries `{bare}`, which is neither a versionId nor an openEHR version id: {}",
                crate::facade::outcome::chain(&error)
            )),
        )
    })?;
    let current = latest_version(client, ehr_id, container).await?;
    let current_tree = current.version_tree_id();
    if current_tree.value() == tree.value() {
        return Ok(current);
    }
    Err(write::refuse(
        &status::Answer::new(
            status::PRECONDITION_FAILED,
            format!(
                "If-Match names version {}, and the current version is {}",
                tree.value(),
                current_tree.value()
            ),
        )
        .with_entity_tag(current_tree.value()),
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
fn stored_identifier(error: &impl std::fmt::Display) -> Refusal {
    reply::refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the identity map holds an openEHR identifier this version cannot read: {error}"
        )),
    )
}
