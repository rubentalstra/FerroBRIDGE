// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Create and update: the two inbound interactions.
//!
//! Create is conditional and idempotent by source identity: a resource whose
//! `id` and `meta.versionId` the identity map already knows resolves to an
//! update of the composition it produced, never a second one
//! (`docs/architecture.md` §4.6). Update needs the map to know the id, because
//! this milestone declares `updateCreate: false`, and it checks the CDR's
//! current `ETag` when the client sends no `If-Match`.

use ferrobridge_openehr::composition::CreateCompositionOutcome;
use ferrobridge_openehr::composition::UpdateCompositionOutcome;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::ids::ObjectVersionId;
use ferrobridge_openehr::ids::VersionedObjectUid;
use ferrobridge_openehr::prefer::Prefer;
use ferrobridge_openehr::prefer::Returned;
use fhirconnect::resolve::program::TemplateId;
use http::HeaderMap;
use http::StatusCode;
use http::Uri;
use http::header;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::composition::composition::Composition;

use ferrobridge_openehr::client::Client;

use crate::facade::Facade;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::engine;
use crate::facade::handlers::Body;
use crate::facade::handlers::Refusal;
use crate::facade::handlers::conditional;
use crate::facade::handlers::read;
use crate::facade::handlers::render;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::derive;
use crate::facade::identity::derive::EntryKey;
use crate::facade::identity::record::CompositionBinding;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::record::SourceVersion;
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
    let client = facade.client_for(headers);

    if let Some(known) = consumed(facade, &inbound)? {
        return resend(facade, &client, program, &inbound, &known, headers).await;
    }
    if let Some(answer) =
        conditional::if_none_exist(facade, &client, resource_type, headers).await?
    {
        return Ok(answer);
    }
    let written = commit_first(facade, &client, program, &inbound).await?;
    answer(facade, program, &written, headers, StatusCode::CREATED)
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
    resend(
        facade,
        &facade.client_for(headers),
        program,
        &inbound,
        &known,
        headers,
    )
    .await
}

/// Runs the update path of a resource the identity map already placed.
async fn resend(
    facade: &Facade,
    client: &Client,
    program: &Loaded,
    inbound: &Inbound,
    known: &ConsumedSource,
    headers: &HeaderMap,
) -> Result<axum::response::Response, Refusal> {
    let ehr_id = EhrId::new(&known.ehr_id).map_err(|error| store_identifier(&error))?;
    let container = VersionedObjectUid::new(&known.versioned_object_uid)
        .map_err(|error| store_identifier(&error))?;
    let preceding = precondition(client, headers, &ehr_id, &container).await?;
    let composition = build(facade, program, inbound)?;
    let rm = strict_read(&composition)?;
    let context = commit::context(
        commit::Change::Modification,
        &facade.settings().system_id,
        composition.template_id(),
    )
    .map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(render::chain(&error)),
        )
    })?;
    let answered = client
        .update_composition(
            &ehr_id,
            &container,
            &preceding,
            &rm,
            &context,
            Prefer::Representation,
        )
        .await
        .map_err(|error| refuse(&status::of_client_error(&error)))?;
    let (version, stored) = match answered {
        UpdateCompositionOutcome::Updated {
            version_id,
            returned,
        } => (version_id, representation(returned, &composition)),
        UpdateCompositionOutcome::Unprocessable(upstream) => {
            return Err(refuse(&status::Answer::new(
                status::UNPROCESSABLE,
                status::diagnostics(&upstream),
            )));
        }
        UpdateCompositionOutcome::PreconditionFailed {
            latest_version_id,
            upstream,
        } => {
            let mut answer =
                status::Answer::new(status::PRECONDITION_FAILED, status::diagnostics(&upstream));
            if let Some(latest) = latest_version_id {
                answer = answer.with_entity_tag(String::from(latest.version_tree_id()));
            }
            return Err(refuse(&answer));
        }
        UpdateCompositionOutcome::NotFound(upstream) => {
            return Err(refuse(&status::Answer::new(
                status::NOT_FOUND,
                status::diagnostics(&upstream),
            )));
        }
        UpdateCompositionOutcome::BadRequest(upstream) => {
            return Err(refuse(&status::Answer::new(
                status::BAD_REQUEST,
                status::diagnostics(&upstream),
            )));
        }
        other => return Err(undocumented(&format!("{other:?}"))),
    };
    let written = record(facade, program, inbound, &ehr_id, &version, &stored)?;
    answer(facade, program, &written, headers, StatusCode::OK)
}

/// Commits the first version of a composition for `inbound`.
async fn commit_first(
    facade: &Facade,
    client: &Client,
    program: &Loaded,
    inbound: &Inbound,
) -> Result<Written, Refusal> {
    let subject = inbound
        .subject(&facade.settings().subject_namespace)
        .map_err(|error| {
            reply::refusal(
                StatusCode::UNPROCESSABLE_ENTITY,
                Issue::error(IssueType::Required)
                    .diagnosing(render::chain(&error))
                    .at(format!("{}.subject", inbound.resource_type())),
            )
        })?;
    let ehr_id = ehr::resolve(
        client,
        facade.store(),
        &subject,
        facade.settings().ehr_policy,
    )
    .await
    .map_err(|error| ehr_refusal(&error))?;
    let composition = build(facade, program, inbound)?;
    let rm = strict_read(&composition)?;
    let context = commit::context(
        commit::Change::Creation,
        &facade.settings().system_id,
        composition.template_id(),
    )
    .map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(render::chain(&error)),
        )
    })?;
    let answered = client
        .create_composition(&ehr_id, &rm, &context, Prefer::Representation)
        .await
        .map_err(|error| refuse(&status::of_client_error(&error)))?;
    let (version, stored) = match answered {
        CreateCompositionOutcome::Created {
            version_id,
            returned,
        } => (version_id, representation(returned, &composition)),
        CreateCompositionOutcome::Unprocessable(upstream) => {
            return Err(refuse(&status::Answer::new(
                status::UNPROCESSABLE,
                status::diagnostics(&upstream),
            )));
        }
        CreateCompositionOutcome::UnknownEhr(upstream) => {
            return Err(refuse(&status::Answer::new(
                status::NOT_FOUND,
                status::diagnostics(&upstream),
            )));
        }
        CreateCompositionOutcome::BadRequest(upstream) => {
            return Err(refuse(&status::Answer::new(
                status::BAD_REQUEST,
                status::diagnostics(&upstream),
            )));
        }
        other => return Err(undocumented(&format!("{other:?}"))),
    };
    record(facade, program, inbound, &ehr_id, &version, &stored)
}

/// What one committed write produced.
#[derive(Debug, Clone)]
pub(crate) struct Written {
    /// The logical id the resource now has.
    pub(crate) id: FhirResourceId,
    /// The EHR the composition lives in.
    pub(crate) ehr_id: EhrId,
    /// The version the commit produced.
    pub(crate) version: ObjectVersionId,
    /// The composition as it now stands.
    pub(crate) composition: CanonicalComposition,
}

/// Records the identity of a committed write, and returns what stands.
fn record(
    facade: &Facade,
    program: &Loaded,
    inbound: &Inbound,
    ehr_id: &EhrId,
    version: &ObjectVersionId,
    composition: &CanonicalComposition,
) -> Result<Written, Refusal> {
    let entry = engine::entry_of(program.program(), composition).ok_or_else(|| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(
                "the committed composition carries no entry of the archetype the program maps",
            ),
        )
    })?;
    // TODO(#186): carry the split occurrence once `hierarchy.split` runs; one
    // program maps one document today, so the occurrence is the first.
    let key = EntryKey::new(version.versioned_object_uid(), entry.path(), 0);
    let map_key = derive::map_key(&key);
    let resource_type = inbound.resource_type();
    let recorded = facade
        .store()
        .internal_of(resource_type, &map_key)
        .map_err(|error| store_refusal(&error))?;
    let id = if let Some(known) = recorded {
        known
    } else {
        let (fresh, _source) = derive::derive(&key, entry.uid());
        facade
            .store()
            .record_internal(resource_type, &map_key, &fresh)
            .map_err(|error| store_refusal(&error))?
    };
    let binding = CompositionBinding {
        ehr_id: String::from(ehr_id.as_str()),
        versioned_object_uid: String::from(version.versioned_object_uid().as_str()),
        template_id: String::from(composition.template_id()),
        resource_type: String::from(resource_type),
        entry_path: String::from(entry.path()),
        split: key.split(),
        context: program.program().context().to_string(),
    };
    let stood = facade
        .store()
        .record_binding(resource_type, &id, &binding)
        .map_err(|error| store_refusal(&error))?;
    if let Some(external) = inbound.id() {
        let source = SourceVersion::new(
            resource_type,
            external.clone(),
            inbound.version_id().map(str::to_owned),
        );
        facade
            .store()
            .record_consumed(
                &source,
                &ConsumedSource {
                    ehr_id: stood.ehr_id.clone(),
                    versioned_object_uid: stood.versioned_object_uid.clone(),
                    internal_id: String::from(id.as_str()),
                    context: stood.context.clone(),
                },
            )
            .map_err(|error| store_refusal(&error))?;
    }
    Ok(Written {
        id,
        ehr_id: ehr_id.clone(),
        version: version.clone(),
        composition: composition.clone(),
    })
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
        &facade.client().config().base_url,
        &written.ehr_id,
        &written.version,
    )?;
    let location = format!(
        "{}/{}/{}/_history/{}",
        facade.settings().base_url.trim_end_matches('/'),
        program.program().resource().as_str(),
        written.id,
        written.version.version_tree_id()
    );
    let response_headers = [
        reply::Header::entity_tag(written.version.version_tree_id()),
        reply::Header::location(&location),
    ];
    match media::Prefer::of(headers) {
        media::Prefer::Minimal => Ok(reply::minimal(status, &response_headers)),
        media::Prefer::Outcome => Ok(reply::issues(
            status,
            &[
                Issue::information(IssueType::Informational).diagnosing(format!(
                    "the resource is committed as version {}",
                    written.version
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
/// "A missing `If-Match` checks the CDR's current `ETag`"
/// (`docs/architecture.md` §4.6): the facade reads the latest version and uses
/// it, so a concurrent writer still produces the `412` the CDR answers.
async fn precondition(
    client: &Client,
    headers: &HeaderMap,
    ehr_id: &EhrId,
    container: &VersionedObjectUid,
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
    facade
        .programs()
        .select(inbound.profiles(), pin.as_ref())
        .map_err(|error| {
            let candidates = facade.programs().candidates(inbound.profiles());
            let issue = if candidates.len() > 1 {
                Issue::error(IssueType::NotSupported).diagnosing(format!(
                    "{}; pin the choice with the templateId parameter, whose candidates are: {}",
                    error,
                    candidates.join(", ")
                ))
            } else {
                Issue::error(IssueType::NotSupported).diagnosing(error.to_string())
            };
            reply::refusal(
                StatusCode::UNPROCESSABLE_ENTITY,
                issue.at("Resource.meta.profile"),
            )
        })
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

/// Runs the engine over one inbound resource.
fn build(
    facade: &Facade,
    program: &Loaded,
    inbound: &Inbound,
) -> Result<CanonicalComposition, Refusal> {
    // NOTE: one instant serves every defaulted time of one ingest
    // (`docs/architecture.md` §12), so the clock is read once here.
    let now = jiff::Timestamp::now().to_string();

    engine::inbound(
        program.program(),
        program.index(),
        inbound.document(),
        &now,
        facade.settings(),
    )
    .map(fhirconnect::engine::outcome::Outcome::into_value)
    .map_err(|error| render::engine_refusal(&error))
}

/// Re-reads a built composition through the strict RM reader.
///
/// "The built composition is re-read through the strict RM reader before it is
/// sent, so a bad document never reaches the CDR"
/// (`docs/architecture.md` §12).
fn strict_read(composition: &CanonicalComposition) -> Result<Composition, Refusal> {
    let text = serde_json::to_string(composition.value()).map_err(|error| {
        reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the built composition could not be written: {error}"
            )),
        )
    })?;
    openehr_its::json::from_canonical_json::<Composition>(&text).map_err(|error| {
        reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Processing).diagnosing(format!(
                "the built composition is no valid openEHR COMPOSITION: {error}"
            )),
        )
    })
}

/// Returns the composition the CDR stored, or the built one.
///
/// The CDR is the authority on what it stored, so its representation wins when
/// `Prefer: return=representation` produced one.
fn representation(
    returned: Returned<Composition>,
    built: &CanonicalComposition,
) -> CanonicalComposition {
    match returned {
        Returned::Representation(stored) => CanonicalComposition::new(
            openehr_its::json::to_canonical_json(stored.as_ref())
                .parse::<serde_json::Value>()
                .unwrap_or_else(|_unparsed| built.value().clone()),
            built.template_id(),
            built.generation(),
        ),
        Returned::Minimal | Returned::Identifier(_) => built.clone(),
    }
}

/// Returns what the map already recorded for this source resource version.
fn consumed(facade: &Facade, inbound: &Inbound) -> Result<Option<ConsumedSource>, Refusal> {
    let Some(external) = inbound.id() else {
        return Ok(None);
    };
    let source = SourceVersion::new(
        inbound.resource_type(),
        external.clone(),
        inbound.version_id().map(str::to_owned),
    );
    facade
        .store()
        .consumed(&source)
        .map_err(|error| store_refusal(&error))
}

/// Returns the refusal one status-table answer renders as.
pub(crate) fn refuse(answer: &status::Answer) -> Refusal {
    let mut headers = Vec::new();
    if let Some(tag) = answer.entity_tag() {
        headers.push(reply::Header::entity_tag(tag));
    }
    if let Some(challenge) = answer.challenge() {
        headers.push(reply::Header::challenge(challenge));
    }
    reply::refusals(answer.status(), &[answer.issue().clone()], &headers)
}

/// Returns the refusal an outcome this version does not read renders as.
fn undocumented(detail: &str) -> Refusal {
    reply::refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the openEHR client answered an outcome this version does not read ({detail})"
        )),
    )
}

/// Returns the refusal an EHR resolution renders as.
fn ehr_refusal(error: &ehr::EhrError) -> Refusal {
    match *error {
        ehr::EhrError::Absent { .. } => reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::NotFound).diagnosing(render::chain(error)),
        ),
        ehr::EhrError::Refused { .. } | ehr::EhrError::Subject { .. } => reply::refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Processing).diagnosing(render::chain(error)),
        ),
        ehr::EhrError::Client { ref source } => refuse(&status::of_client_error(source)),
        _ => reply::refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(render::chain(error)),
        ),
    }
}

/// Returns the refusal an identity-store failure renders as.
pub(crate) fn store_refusal(error: &crate::facade::identity::store::StoreError) -> Refusal {
    reply::refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(render::chain(&error)),
    )
}

/// Returns the refusal a malformed stored identifier renders as.
fn store_identifier(error: &ferrobridge_openehr::ids::IdError) -> Refusal {
    reply::refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the identity map holds an openEHR identifier this version cannot read: {error}"
        )),
    )
}
