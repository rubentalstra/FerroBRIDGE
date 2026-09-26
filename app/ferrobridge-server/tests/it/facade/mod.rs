// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR facade on the wire, through the library run path.
//!
//! Every case drives the shipped router over a `wiremock` CDR, so the stack
//! under test is the stack the binary serves: the middleware, the media-type
//! guard, the program selection, the engine, the identity map and the status
//! table.

mod capability;
mod conditional;
mod examples;
mod read;
mod reconcile;
mod replay;
pub(crate) mod stub;
mod transaction;
mod validate;
mod vread;
mod wire;
mod write;

use crate::support;
use axum::Router;
use axum::body::Body;
use ferrobridge_server::cdr::CdrClient;
use ferrobridge_server::cdr::config::CdrConfig;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::Settings;
use ferrobridge_server::facade::ehr::Policy;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::identity::store::Store;
use ferrobridge_server::facade::programs;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
use ferrobridge_server::state::OperationsLane;
use fhirconnect::operations::programs::ProgramSet;
use http::Request;
use http::Response;
use http::StatusCode;
use http::header;
use std::error::Error as StdError;
use std::sync::Arc;
use tower::ServiceExt as _;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use stub::mount_create;
use stub::mount_ehr;

/// The fixture directory this suite loads its mapping set from.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/fixtures");

/// The profile the fixture context claims.
const PROFILE: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-facade-diagnosis";

/// The EHR every case writes into.
pub(crate) const EHR_ID: &str = "bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11";

/// The version container the first commit produces.
const CONTAINER: &str = "8849182c-82ad-4088-a07f-48ead4180515";

/// The version the first commit produces.
const VERSION_ONE: &str = "8849182c-82ad-4088-a07f-48ead4180515::ferrobridge.test::1";

/// The version a second commit produces.
const VERSION_TWO: &str = "8849182c-82ad-4088-a07f-48ead4180515::ferrobridge.test::2";

/// The contribution the stub CDR answers a transaction with.
const CONTRIBUTION: &str = "7b0a4c2e-0000-4000-8000-00000000000c";

/// The FHIR service base the facade answers under.
const BASE_URL: &str = "http://ferrobridge.invalid/fhir";

/// The `$validate` path of the fixture type.
const VALIDATE: &str = "/fhir/Condition/$validate";

/// Everything one case drives.
struct Harness {
    /// The stub CDR, kept alive for the case's lifetime.
    cdr: MockServer,
    /// The identity map, so a case can assert what was recorded.
    store: Arc<MemoryStore>,
    /// The facade the router is built over.
    facade: Arc<Facade>,
}

impl Harness {
    /// Returns the application under test.
    fn app(&self) -> Router {
        let state =
            AppState::with_health(Registry::default()).with_facade(Arc::clone(&self.facade));
        ferrobridge_server::router(Arc::new(state), &server_settings())
    }

    /// Returns the application under test with the FHIRconnect operations
    /// lane served beside the facade.
    fn app_with_operations(&self) -> Router {
        let lane = OperationsLane::new(ProgramSet::new(), "Device/ferrobridge-test");
        let state = AppState::with_health(Registry::default())
            .with_facade(Arc::clone(&self.facade))
            .serving_operations(lane);
        ferrobridge_server::router(Arc::new(state), &server_settings())
    }

    /// Returns the canonical composition the synthetic Condition maps to.
    fn composition(&self) -> serde_json::Value {
        self.composition_of(&condition())
    }

    /// Returns the canonical composition `resource` maps to.
    fn composition_of(&self, resource: &serde_json::Value) -> serde_json::Value {
        let loaded = self
            .facade
            .programs()
            .by_context("ferrobridge_facade.context")
            .expect("the fixture context compiled");
        let document: fhir_types::codec::Value = serde_json::from_value(resource.clone())
            .expect("the synthetic Condition reads as a lexical document");
        let built = ferrobridge_server::facade::engine::inbound(
            loaded.program(),
            loaded.index(),
            &document,
            "2026-09-15T09:00:00Z",
            &settings(),
        )
        .expect("the synthetic Condition maps");
        built.into_value().into_value()
    }

    /// Returns the body of the last request the CDR received on `path`.
    async fn sent(&self, method: &str, path_prefix: &str) -> Option<serde_json::Value> {
        self.cdr
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .rev()
            .find(|request| {
                request.method.as_str() == method && request.url.path().starts_with(path_prefix)
            })
            .and_then(|request| serde_json::from_slice(&request.body).ok())
    }

    /// Returns how many requests the CDR received on `path`.
    async fn received(&self, method: &str, path_prefix: &str) -> usize {
        self.cdr
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| {
                request.method.as_str() == method && request.url.path().starts_with(path_prefix)
            })
            .count()
    }
}

/// Returns the server settings this suite drives the middleware with.
///
/// The body ceiling is wider than the shared one because a transaction Bundle
/// carries more than one resource.
fn server_settings() -> ferrobridge_server::config::ServerSettings {
    ferrobridge_server::config::ServerSettings {
        body_limit: 1024 * 1024,
        ..support::settings()
    }
}

/// Returns the in-memory store as the trait object the facade holds.
pub(crate) fn handle(store: &Arc<MemoryStore>) -> Arc<dyn Store> {
    let held: Arc<MemoryStore> = Arc::clone(store);
    held
}

/// Returns the CDR client one case calls through.
pub(crate) fn client(cdr: &MockServer) -> CdrClient {
    let base = format!("{}/", cdr.uri()).parse().expect("a legal base URL");
    CdrClient::new(&CdrConfig::new(base)).expect("the client builds")
}

/// Returns the facade settings every case runs with.
pub(crate) fn settings() -> Settings {
    Settings {
        base_url: String::from(BASE_URL),
        ehr_policy: Policy::Existing,
        subject_namespace: String::from("http://example.org/fhir"),
        system_id: String::from("ferrobridge.test"),
        language: String::from("en"),
        territory: String::from("GB"),
    }
}

/// Returns a harness whose CDR serves the fixture template.
async fn harness() -> Harness {
    harness_with(settings()).await
}

/// Returns a harness whose facade runs with `facade_settings`.
async fn harness_with(facade_settings: Settings) -> Harness {
    let cdr = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(
            "/definition/template/adl1.4/ferrobridge.diagnose.v1",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(ferrobridge_testkit::fixtures::DIAGNOSE_OPT),
        )
        .mount(&cdr)
        .await;
    let set = programs::read_set(std::path::Path::new(FIXTURES)).expect("the mapping set loads");
    let templates = programs::fetch_templates(&set, &client(&cdr))
        .await
        .expect("the template fetch answers");
    let compiled = programs::compile_set(&set, &templates).expect("the mapping set compiles");
    let store = Arc::new(MemoryStore::new());
    let facade = Arc::new(Facade::new(
        compiled,
        handle(&store),
        client(&cdr),
        facade_settings,
    ));
    Harness { cdr, store, facade }
}

/// Returns the canonical JSON of an EHR whose `ehr_id` is [`EHR_ID`].
pub(crate) fn ehr_body() -> String {
    serde_json::json!({
        "_type": "EHR",
        "system_id": { "_type": "HIER_OBJECT_ID", "value": "ferrobridge.test" },
        "ehr_id": { "_type": "HIER_OBJECT_ID", "value": EHR_ID },
        "ehr_status": {
            "_type": "OBJECT_REF",
            "namespace": "local",
            "type": "EHR_STATUS",
            "id": { "_type": "HIER_OBJECT_ID", "value": "0a1b2c3d-0000-4000-8000-00000000000a" }
        },
        "ehr_access": {
            "_type": "OBJECT_REF",
            "namespace": "local",
            "type": "EHR_ACCESS",
            "id": { "_type": "HIER_OBJECT_ID", "value": "0a1b2c3d-0000-4000-8000-00000000000b" }
        },
        "time_created": { "_type": "DV_DATE_TIME", "value": "2026-09-15T09:00:00Z" }
    })
    .to_string()
}

/// Sends `request` through `app` and reads the status and the body.
async fn call(
    app: Router,
    request: Request<Body>,
) -> Result<(StatusCode, serde_json::Value), Box<dyn StdError>> {
    let response = raw(app, request).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        // A body this suite cannot read as JSON is a failure worth seeing, so
        // it reaches the assertion as text rather than as a parse error.
        serde_json::from_slice(&bytes).unwrap_or_else(|_unparsed| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    Ok((status, body))
}

/// Sends `request` through `app` and returns the whole response.
async fn raw(app: Router, request: Request<Body>) -> Result<Response<Body>, Box<dyn StdError>> {
    Ok(app.oneshot(request).await?)
}

/// Returns the synthetic Condition, with [`PROFILE`] claimed.
fn condition() -> serde_json::Value {
    let mut resource: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)
            .expect("the synthetic Condition reads");
    resource["meta"] = serde_json::json!({ "profile": [PROFILE] });
    resource
}

/// Returns a `POST [base]/Condition` carrying `body`.
fn post_condition(body: &serde_json::Value) -> Request<Body> {
    Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(body.to_string()))
        .expect("a legal request")
}

/// Returns the first issue of an `OperationOutcome` body.
fn first_issue(body: &serde_json::Value) -> &serde_json::Value {
    assert_eq!(
        Some("OperationOutcome"),
        body["resourceType"].as_str(),
        "the facade answered {body} where an OperationOutcome belongs"
    );
    &body["issue"][0]
}

/// Creates the fixture Condition and returns the logical id it was given.
async fn create_one(harness: &Harness) -> Result<String, Box<dyn StdError>> {
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    Ok(body["id"]
        .as_str()
        .expect("the created resource carries an id")
        .to_owned())
}

/// Sends a `GET` of `path` and returns the status, the `ETag` and the body.
async fn get_tagged(
    harness: &Harness,
    path: &str,
) -> Result<(StatusCode, Option<String>, serde_json::Value), Box<dyn StdError>> {
    let response = raw(harness.app(), Request::get(path).body(Body::empty())?).await?;
    let status = response.status();
    let tag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = serde_json::from_slice(&bytes)?;
    Ok((status, tag, body))
}

/// Returns the synthetic Condition with its code text changed, content the
/// fixture program maps.
fn revised_condition() -> serde_json::Value {
    let mut resource = condition();
    resource["code"]["text"] = serde_json::json!("Synthetic problem one, revised");
    resource
}

/// Returns the synthetic Condition at the sender's `meta.versionId`
/// `version`.
fn condition_at(version: &str) -> serde_json::Value {
    let mut resource = condition();
    resource["meta"]["versionId"] = serde_json::json!(version);
    resource
}

/// Posts `body` as a create and returns the status, the `Location` header and
/// the body.
async fn post_located(
    harness: &Harness,
    body: &serde_json::Value,
) -> Result<(StatusCode, Option<String>, serde_json::Value), Box<dyn StdError>> {
    let response = raw(harness.app(), post_condition(body)).await?;
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = serde_json::from_slice(&bytes)?;
    Ok((status, location, body))
}

/// Returns how many composition writes of each kind the CDR received: the
/// creates, the updates of [`CONTAINER`] and the contributions.
async fn writes(harness: &Harness) -> (usize, usize, usize) {
    (
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        harness
            .received("PUT", &format!("/ehr/{EHR_ID}/composition/{CONTAINER}"))
            .await,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
    )
}

/// Returns a conditional create of the synthetic Condition under another
/// sender `id`, so the source lookup meets nothing and the search decides.
fn conditional_create(if_none_exist: &str) -> Result<Request<Body>, Box<dyn StdError>> {
    conditional_create_as("a-sender-id-the-map-has-not-seen", if_none_exist)
}

/// Returns the conditional create of [`conditional_create`] under the sender
/// `id` `sender_id`.
fn conditional_create_as(
    sender_id: &str,
    if_none_exist: &str,
) -> Result<Request<Body>, Box<dyn StdError>> {
    let mut fresh = condition();
    fresh["id"] = serde_json::json!(sender_id);
    Ok(Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", if_none_exist)
        .body(Body::from(fresh.to_string()))?)
}

/// Returns a transaction Bundle carrying one `POST` entry per resource.
fn transaction_of(resources: &[(&str, serde_json::Value)]) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = resources
        .iter()
        .map(|(full_url, resource)| {
            serde_json::json!({
                "fullUrl": full_url,
                "resource": resource,
                "request": { "method": "POST", "url": "Condition" }
            })
        })
        .collect();
    serde_json::json!({ "resourceType": "Bundle", "type": "transaction", "entry": entries })
}

/// Posts `bundle` to the system endpoint and returns the answer.
async fn post_transaction(
    harness: &Harness,
    bundle: &serde_json::Value,
) -> Result<(StatusCode, serde_json::Value), Box<dyn StdError>> {
    call(
        harness.app(),
        Request::post("/fhir")
            .header(header::CONTENT_TYPE, "application/fhir+json")
            .body(Body::from(bundle.to_string()))?,
    )
    .await
}

/// Returns the consumed-source key of the synthetic Condition at `version`.
fn condition_source_at(
    version: &str,
) -> Result<ferrobridge_server::facade::identity::record::SourceVersion, Box<dyn StdError>> {
    Ok(
        ferrobridge_server::facade::identity::record::SourceVersion::new(
            "Condition",
            ferrobridge_server::facade::identity::ExternalResourceId::new(
                "ferrobridge-synthetic-condition-1",
            )?,
            Some(String::from(version)),
        ),
    )
}

/// Returns the `diagnostics` of every issue of an `OperationOutcome`.
fn diagnostics_of(body: &serde_json::Value) -> Vec<&str> {
    body["issue"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .filter_map(|issue| issue["diagnostics"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Asserts that the stub CDR received no write of any kind.
async fn assert_nothing_written(harness: &Harness) {
    for method in ["POST", "PUT", "DELETE"] {
        assert_eq!(
            0,
            harness.received(method, "/ehr").await,
            "the dry run sent a {method} to the CDR"
        );
    }
}

/// Returns the facade settings with `policy` in place of the default one.
fn settings_with(policy: Policy) -> Settings {
    Settings {
        ehr_policy: policy,
        ..settings()
    }
}
