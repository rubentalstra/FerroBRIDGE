// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HTTP surface of the two FHIRconnect operations.
//!
//! The draft REST API chapter of the FHIRconnect specification (pull request
//! #93, vendored at its pinned commit) defines `POST [base]/$tofhir` and
//! `POST [base]/$toopenehr` against the service base, both following the FHIR
//! R4 operations framework, plus a direct form outside the FHIR
//! implementation guide where the payload is the body itself. All four are
//! served here.
//!
//! Every refusal answers an `OperationOutcome` in `application/fhir+json`, and
//! a body in any other media type answers `415`. The request log carries the
//! matched route and never a body.

use std::sync::Arc;

use axum::Router;
use axum::extract::Query;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use fhir_types::codec::Json;
use fhir_types::codec::Path;
use fhir_types::r4::operation_outcome::OperationOutcome;
use fhir_types::r4::parameters::Parameters;
use fhir_types::r4::parameters::ParametersParameter;
use fhir_types::r4::parameters::ParametersParameterValue;
use fhir_types::r4::resource::Resource;
use fhir_types::r4::schema::SCHEMAS;
use fhirconnect::engine::traverse::NoMappingFunctions;
use fhirconnect::operations::TO_FHIR;
use fhirconnect::operations::TO_OPENEHR;
use fhirconnect::operations::contract::CompositionPayload;
use fhirconnect::operations::contract::Format;
use fhirconnect::operations::contract::ToFhirRequest;
use fhirconnect::operations::contract::ToOpenehrRequest;
use fhirconnect::operations::error::OperationError;
use fhirconnect::operations::issues;
use fhirconnect::operations::run;
use fhirconnect::resolve::program::TemplateId;
use http::StatusCode;
use http::header::CONTENT_TYPE;
use serde::Deserialize;

use crate::state::AppState;
use crate::state::OperationsLane;

/// The FHIR JSON media type, the one both operations exchange.
pub const FHIR_JSON: &str = "application/fhir+json";

/// The media type the draft defines for an unwrapped openEHR payload.
pub const OPENEHR_JSON: &str = fhirconnect::operations::OPENEHR_JSON;

/// The route prefix the FHIR surface is served under.
pub const PREFIX: &str = "/fhir";

/// The query parameters both operations accept.
///
/// "In addition, context fields MAY be passed as query parameters instead of
/// as `Parameters` parts, provided their values are short enough to stay
/// within URL length limits" (`engine/rest-api.adoc` §Query parameters). A
/// structured value never travels here.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OperationQuery {
    /// Pins the mapping to a specific openEHR template.
    #[serde(rename = "templateId")]
    pub template_id: Option<String>,
    /// The serialization of the returned composition.
    pub format: Option<String>,
    /// The openEHR EHR identifier associated with the composition.
    pub ehr_id: Option<String>,
}

/// Returns the routes of the two operations and their direct forms.
///
/// The enveloped operations are the interoperable contract; the two bare
/// routes are the direct form, which the chapter marks as "a deliberate
/// deviation from the FHIR Operations framework" and keeps out of the
/// FHIRconnect FHIR implementation guide.
pub fn router(state: Arc<AppState>) -> Router {
    // TODO(#194): declare the two operations in the CapabilityStatement the
    // facade publishes, and leave the two direct forms out of it.
    Router::new()
        .route("/fhir/$tofhir", post(to_fhir))
        .route("/fhir/$toopenehr", post(to_openehr))
        .route("/fhir/tofhir", post(to_fhir_direct))
        .route("/fhir/toopenehr", post(to_openehr_direct))
        .with_state(state)
}

/// `POST /fhir/$tofhir`: a `Parameters` in, a `Bundle` out.
async fn to_fhir(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OperationQuery>,
    headers: http::HeaderMap,
    body: String,
) -> Response {
    let Some(lane) = state.operations() else {
        return unavailable();
    };
    let settings = lane.settings();
    if let Some(refusal) = fhir_media_type(&headers) {
        return refusal;
    }
    match tofhir_answer(lane, &settings, &query, &body) {
        Ok(response) => response,
        Err(error) => refused(&error),
    }
}

/// `POST /fhir/$toopenehr`: a `Bundle` or a `Parameters` in, a `Parameters`
/// out.
async fn to_openehr(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OperationQuery>,
    headers: http::HeaderMap,
    body: String,
) -> Response {
    let Some(lane) = state.operations() else {
        return unavailable();
    };
    let settings = lane.settings();
    if let Some(refusal) = fhir_media_type(&headers) {
        return refusal;
    }
    match toopenehr_answer(lane, &settings, &query, &body) {
        Ok(response) => response,
        Err(error) => refused(&error),
    }
}

/// `POST /fhir/tofhir`: the composition as the body, a `Bundle` out.
///
/// The direct form of the chapter's §Direct payload invocation, outside the
/// FHIR implementation guide.
async fn to_fhir_direct(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OperationQuery>,
    headers: http::HeaderMap,
    body: String,
) -> Response {
    let Some(lane) = state.operations() else {
        return unavailable();
    };
    let settings = lane.settings();
    if let Some(refusal) = media_type(&headers, &[OPENEHR_JSON]) {
        return refusal;
    }
    let answer = CompositionPayload::parse(&body)
        .map(ToFhirRequest::new)
        .map(|request| with_query(request, &query))
        .and_then(|request| {
            run::to_fhir(
                lane.programs(),
                &SCHEMAS,
                &NoMappingFunctions,
                &settings,
                &request,
            )
        });
    match answer {
        Ok(response) => fhir_resource(
            StatusCode::OK,
            &Resource::Bundle(Box::new(response.into_bundle())),
        ),
        Err(error) => refused(&error),
    }
}

/// `POST /fhir/toopenehr`: a FHIR body, the composition itself out.
///
/// The direct form of the chapter's §Direct payload invocation, outside the
/// FHIR implementation guide.
async fn to_openehr_direct(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OperationQuery>,
    headers: http::HeaderMap,
    body: String,
) -> Response {
    let Some(lane) = state.operations() else {
        return unavailable();
    };
    let settings = lane.settings();
    if let Some(refusal) = media_type(&headers, &[FHIR_JSON, "application/json"]) {
        return refusal;
    }
    match toopenehr_request(&body, &query).and_then(|request| {
        run::to_openehr(
            lane.programs(),
            &SCHEMAS,
            &NoMappingFunctions,
            &settings,
            &request,
        )
    }) {
        Ok(response) => (
            StatusCode::OK,
            [(CONTENT_TYPE, OPENEHR_JSON)],
            String::from(response.composition()),
        )
            .into_response(),
        Err(error) => refused(&error),
    }
}

/// Runs `$tofhir` over an enveloped request.
fn tofhir_answer(
    lane: &OperationsLane,
    settings: &run::Settings,
    query: &OperationQuery,
    body: &str,
) -> Result<Response, OperationError> {
    let mut parameters = fhirconnect::operations::contract::parameters_from_json(TO_FHIR, body)?;
    merge_query(&mut parameters, query);
    let request = ToFhirRequest::from_parameters(&parameters)?;
    let answer = run::to_fhir(
        lane.programs(),
        &SCHEMAS,
        &NoMappingFunctions,
        settings,
        &request,
    )?;
    Ok(fhir_resource(
        StatusCode::OK,
        &Resource::Bundle(Box::new(answer.into_bundle())),
    ))
}

/// Runs `$toopenehr` over an enveloped request.
fn toopenehr_answer(
    lane: &OperationsLane,
    settings: &run::Settings,
    query: &OperationQuery,
    body: &str,
) -> Result<Response, OperationError> {
    let request = toopenehr_request(body, query)?;
    let answer = run::to_openehr(
        lane.programs(),
        &SCHEMAS,
        &NoMappingFunctions,
        settings,
        &request,
    )?;
    Ok(fhir_resource(
        StatusCode::OK,
        &Resource::Parameters(Box::new(answer.to_parameters())),
    ))
}

/// Reads a `$toopenehr` request from a `Bundle` or a `Parameters` body.
///
/// The chapter's prose puts the `Bundle` in the body directly while
/// `ToOpenEhr.fsh` declares it as an `in` parameter, so both forms are
/// accepted and the difference is recorded as an upstream report.
fn toopenehr_request(
    body: &str,
    query: &OperationQuery,
) -> Result<ToOpenehrRequest, OperationError> {
    // NOTE: `engine/rest-api.adoc` §$toopenehr Input and `ToOpenEhr.fsh`
    // describe two different bodies, so both are read (#192).
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|source| OperationError::Payload { source })?;
    let fhir_types::codec::Value::Object(object) = fhir_types::codec::Value::from_serde_json(value)
    else {
        return Err(OperationError::NotAnObject { what: "body" });
    };
    let is_parameters = matches!(
        object.get("resourceType"),
        Some(fhir_types::codec::Value::String(name)) if name == "Parameters"
    );
    let request = if is_parameters {
        let parameters =
            Parameters::from_json(&object, &mut Path::root("Parameters")).map_err(|source| {
                OperationError::Body {
                    operation: TO_OPENEHR,
                    expected: "Parameters",
                    source: Box::new(source),
                }
            })?;
        ToOpenehrRequest::from_parameters(&parameters)?
    } else {
        let bundle = fhir_types::r4::bundle::Bundle::from_json(&object, &mut Path::root("Bundle"))
            .map_err(|source| OperationError::Body {
                operation: TO_OPENEHR,
                expected: "Bundle",
                source: Box::new(source),
            })?;
        ToOpenehrRequest::new(bundle)
    };
    let mut request = request;
    if request.template_id().is_none()
        && let Some(ref template) = query.template_id
    {
        request = request.with_template_id(TemplateId::new(template.as_str()));
    }
    if let Some(ref code) = query.format {
        let asked = Format::parse(code)?;
        if request.format() == Format::default() {
            request = request.with_format(asked);
        }
    }
    Ok(request)
}

/// Folds the query parameters into the `Parameters` the body carried.
///
/// "The two forms are equivalent ... Where the same field is supplied both in
/// the body and as a query parameter, the body takes precedence"
/// (`engine/rest-api.adoc` §Query parameters), so a query value is added only
/// where the body carries none and the contract then reads one parameter set.
fn merge_query(parameters: &mut Parameters, query: &OperationQuery) {
    let carries = |name: &str| {
        parameters
            .parameter
            .iter()
            .any(|parameter| parameter.name.value.as_deref() == Some(name))
    };
    if let Some(ref template) = query.template_id
        && !carries("templateId")
    {
        parameters.parameter.push(ParametersParameter {
            name: fhir_types::r4::primitives::String::from("templateId"),
            value: Some(ParametersParameterValue::String(
                fhir_types::r4::primitives::String::from(template.as_str()),
            )),
            ..ParametersParameter::default()
        });
    }
    let Some(ref id) = query.ehr_id else {
        return;
    };
    let context = parameters
        .parameter
        .iter_mut()
        .find(|parameter| parameter.name.value.as_deref() == Some("context"));
    let part = ParametersParameter {
        name: fhir_types::r4::primitives::String::from("ehr_id"),
        value: Some(ParametersParameterValue::String(
            fhir_types::r4::primitives::String::from(id.as_str()),
        )),
        ..ParametersParameter::default()
    };
    match context {
        Some(group) => {
            if !group
                .part
                .iter()
                .any(|carried| carried.name.value.as_deref() == Some("ehr_id"))
            {
                group.part.push(part);
            }
        }
        None => parameters.parameter.push(ParametersParameter {
            name: fhir_types::r4::primitives::String::from("context"),
            part: vec![part],
            ..ParametersParameter::default()
        }),
    }
}

/// Applies the query parameters the direct `$tofhir` form may carry.
///
/// The direct form carries the composition as the body, so the query is the
/// only place a template or an EHR identifier can travel.
fn with_query(request: ToFhirRequest, query: &OperationQuery) -> ToFhirRequest {
    let mut request = request;
    if request.template_id().is_none()
        && let Some(ref template) = query.template_id
    {
        request = request.with_template_id(TemplateId::new(template.as_str()));
    }
    if let Some(ref id) = query.ehr_id {
        let carried = request.context().cloned().unwrap_or_default();
        if carried.ehr_id().is_none() {
            request = request.with_context(carried.with_ehr_id(id.as_str()));
        }
    }
    request
}

/// Returns the response one refusal answers with.
///
/// The status is FerroBRIDGE's own: the draft fixes none. A request the caller
/// can correct is `400`, a mapping that cannot be performed over a
/// well-formed request is `422`
/// (<https://hl7.org/fhir/R4/http.html#4.4.1.2>).
fn refused(error: &OperationError) -> Response {
    let status = match *error {
        OperationError::Mapping { .. }
        | OperationError::Serialization { .. }
        | OperationError::Mapped { .. }
        | OperationError::Encode { .. }
        | OperationError::Unidentified { .. }
        | OperationError::SeveralSubjects { .. }
        | OperationError::SeveralSubjectResources { .. }
        | OperationError::NoSubjectResource { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::BAD_REQUEST,
    };
    outcome(status, &error.outcome())
}

/// Returns the `503` a call takes when the lane is off.
fn unavailable() -> Response {
    let reported = OperationOutcome {
        issue: vec![issues::issue(
            issues::ERROR,
            "not-supported",
            "the FHIRconnect operations are not served: no mapping set is configured",
        )],
        ..OperationOutcome::default()
    };
    outcome(StatusCode::SERVICE_UNAVAILABLE, &reported)
}

/// Refuses a body whose media type the route does not accept.
fn media_type(headers: &http::HeaderMap, accepted: &[&str]) -> Option<Response> {
    let carried = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_owned());
    match carried {
        Some(ref value) if accepted.contains(&value.as_str()) => None,
        _ => {
            let reported = OperationOutcome {
                issue: vec![issues::issue(
                    issues::ERROR,
                    "not-supported",
                    &format!("this route reads {}", accepted.join(" or ")),
                )],
                ..OperationOutcome::default()
            };
            Some(outcome(StatusCode::UNSUPPORTED_MEDIA_TYPE, &reported))
        }
    }
}

/// Refuses a body that is not FHIR JSON.
///
/// "The request and response bodies of both operations are FHIR resources ...
/// and are exchanged as `application/fhir+json`" (`engine/rest-api.adoc`
/// §Media types); `application/json` is accepted beside it because FHIR itself
/// names both (<https://hl7.org/fhir/R4/http.html#mime-type>).
fn fhir_media_type(headers: &http::HeaderMap) -> Option<Response> {
    media_type(headers, &[FHIR_JSON, "application/json"])
}

/// Renders an `OperationOutcome` as the body of `status`.
fn outcome(status: StatusCode, reported: &OperationOutcome) -> Response {
    fhir_resource(
        status,
        &Resource::OperationOutcome(Box::new(reported.clone())),
    )
}

/// Renders a FHIR resource as `application/fhir+json`.
fn fhir_resource(status: StatusCode, resource: &Resource) -> Response {
    let rendered = Json::to_json(resource)
        .ok()
        .map(fhir_types::codec::Value::Object)
        .and_then(|value| value.to_serde_json(&mut Path::root("Resource")).ok())
        .map(|value| value.to_string());
    match rendered {
        Some(body) => (status, [(CONTENT_TYPE, FHIR_JSON)], body).into_response(),
        // NOTE: no specification governs this: our own design, a resource the
        // codec cannot render is an internal fault, reported as one rather
        // than as an empty body.
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(CONTENT_TYPE, FHIR_JSON)],
            String::from(
                r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"exception"}]}"#,
            ),
        )
            .into_response(),
    }
}
