// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR facade on the wire, through the library run path.
//!
//! Every case drives the shipped router over a `wiremock` CDR, so the stack
//! under test is the stack the binary serves: the middleware, the media-type
//! guard, the program selection, the engine, the identity map and the status
//! table (`docs/architecture.md` §4.6).

use crate::support;
use axum::Router;
use axum::body::Body;
use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::config::Config;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::Settings;
use ferrobridge_server::facade::ehr::Policy;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::identity::store::Store;
use ferrobridge_server::facade::programs;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
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

/// The fixture directory this suite loads its mapping set from.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/fixtures");

/// The profile the fixture context claims.
const PROFILE: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-facade-diagnosis";

/// The EHR every case writes into.
const EHR_ID: &str = "bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11";

/// The version container the first commit produces.
const CONTAINER: &str = "8849182c-82ad-4088-a07f-48ead4180515";

/// The version the first commit produces.
const VERSION_ONE: &str = "8849182c-82ad-4088-a07f-48ead4180515::ferrobridge.test::1";

/// The version a second commit produces.
const VERSION_TWO: &str = "8849182c-82ad-4088-a07f-48ead4180515::ferrobridge.test::2";

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

    /// Returns the canonical composition the synthetic Condition maps to.
    fn composition(&self) -> serde_json::Value {
        let loaded = self
            .facade
            .programs()
            .by_context("ferrobridge_facade.context")
            .expect("the fixture context compiled");
        let document: fhir_types::codec::Value = serde_json::from_value(condition())
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
fn handle(store: &Arc<MemoryStore>) -> Arc<dyn Store> {
    let held: Arc<MemoryStore> = Arc::clone(store);
    held
}

/// Returns the CDR client one case calls through.
fn client(cdr: &MockServer) -> Client {
    let base = format!("{}/", cdr.uri()).parse().expect("a legal base URL");
    Client::new(Config::new(base)).expect("the client builds")
}

/// Returns the facade settings every case runs with.
fn settings() -> Settings {
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
        settings(),
    ));
    Harness { cdr, store, facade }
}

/// Mounts the EHR lookup that answers with [`EHR_ID`].
async fn mount_ehr(cdr: &MockServer) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ehr_body()))
        .mount(cdr)
        .await;
}

/// Mounts the composition create that answers `201` with `body`.
async fn mount_create(cdr: &MockServer, version: &str, body: &serde_json::Value) {
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(body.to_string()),
        )
        .mount(cdr)
        .await;
}

/// Mounts the composition read that answers `200` with `body`.
async fn mount_read(cdr: &MockServer, version: &str, body: &serde_json::Value) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(body.to_string()),
        )
        .mount(cdr)
        .await;
}

/// Mounts the composition update that answers with `response`.
async fn mount_update(cdr: &MockServer, response: ResponseTemplate) {
    Mock::given(matchers::method("PUT"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .respond_with(response)
        .mount(cdr)
        .await;
}

/// Returns the canonical JSON of an EHR whose `ehr_id` is [`EHR_ID`].
fn ehr_body() -> String {
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

#[tokio::test]
async fn a_disabled_facade_mounts_no_route_and_answers_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let app = ferrobridge_server::router(support::state(), &support::settings());
    let (status, _) = call(app, Request::get("/fhir/metadata").body(Body::empty())?).await?;
    assert_eq!(
        StatusCode::NOT_FOUND,
        status,
        "capability is not authorisation: an unmounted facade is 404, never 403"
    );
    Ok(())
}

#[tokio::test]
async fn the_capability_statement_names_exactly_the_loaded_programs()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let response = raw(
        harness.app(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    assert_eq!(
        Some("application/fhir+json"),
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(Some("CapabilityStatement"), body["resourceType"].as_str());
    assert_eq!(Some("4.0.1"), body["fhirVersion"].as_str());
    let resources = body["rest"][0]["resource"]
        .as_array()
        .expect("the statement names its resource types");
    assert_eq!(1, resources.len(), "{body}");
    let condition = &resources[0];
    assert_eq!(Some("Condition"), condition["type"].as_str());
    assert_eq!(Some(false), condition["updateCreate"].as_bool());
    assert_eq!(Some(true), condition["conditionalCreate"].as_bool());
    assert_eq!(Some(true), condition["conditionalUpdate"].as_bool());
    let interactions: Vec<&str> = condition["interaction"]
        .as_array()
        .expect("the type names its interactions")
        .iter()
        .filter_map(|entry| entry["code"].as_str())
        .collect();
    assert_eq!(vec!["create", "read", "update"], interactions);
    assert!(
        condition["searchParam"].is_null(),
        "search is not declared until it is implemented: {condition}"
    );
    assert_eq!(Some("validate"), condition["operation"][0]["name"].as_str());
    let system: Vec<&str> = body["rest"][0]["interaction"]
        .as_array()
        .expect("the system level names its interactions")
        .iter()
        .filter_map(|entry| entry["code"].as_str())
        .collect();
    assert_eq!(vec!["transaction"], system);
    Ok(())
}

#[tokio::test]
async fn a_type_with_no_program_is_not_supported_on_the_wire() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/Observation/abc").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(
        Some("not-supported"),
        first_issue(&body)["code"].as_str(),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_body_media_type_this_server_does_not_read_is_four_hundred_and_fifteen()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+xml")
        .body(Body::from("<Condition/>"))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::UNSUPPORTED_MEDIA_TYPE, status);
    assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_malformed_count_is_four_hundred_invalid() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/metadata?_count=many").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(
        Some("invalid"),
        first_issue(&body)["code"].as_str(),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_property_is_four_hundred_structure() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut resource = condition();
    resource["notAnElement"] = serde_json::json!("x");
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(
        Some("structure"),
        first_issue(&body)["code"].as_str(),
        "an unknown property is refused, never dropped: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_resource_no_program_claims_is_four_hundred_and_twenty_two()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut resource = condition();
    resource["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    let issue = first_issue(&body);
    assert_eq!(Some("not-supported"), issue["code"].as_str());
    assert!(
        issue["diagnostics"]
            .as_str()
            .is_some_and(|text| text.contains("http://example.org/other")),
        "the refusal names the profiles it saw: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_answers_two_hundred_and_one_with_location_and_etag()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let response = raw(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::CREATED, response.status());
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("a create carries Location")
        .to_owned();
    assert_eq!(
        Some("W/\"1\""),
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let id = body["id"].as_str().expect("the resource carries an id");
    assert_eq!(
        format!("{BASE_URL}/Condition/{id}/_history/1"),
        location,
        "Location names the FHIR resource, never the composition"
    );
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert!(
        body["meta"]["source"]
            .as_str()
            .is_some_and(|source| source.contains(VERSION_ONE)),
        "meta.source names the openEHR version uid: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_records_the_identity_it_assigned() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    let internal = ferrobridge_server::facade::identity::FhirResourceId::new(&id)?;
    let binding = harness
        .store
        .binding_of("Condition", &internal)?
        .expect("the create recorded where the resource lives");
    assert_eq!(EHR_ID, binding.ehr_id);
    assert_eq!(CONTAINER, binding.versioned_object_uid);
    assert_eq!("ferrobridge_facade.context", binding.context);
    Ok(())
}

#[tokio::test]
async fn a_create_records_the_source_resource_in_the_feeder_audit() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    create_one(&harness).await?;
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/composition"))
        .await
        .ok_or("the create sent a composition")?;
    let audit = &sent["feeder_audit"];
    let item = &audit["originating_system_item_ids"][0];
    assert_eq!(
        Some("ferrobridge-synthetic-condition-1"),
        item["id"].as_str(),
        "the source resource id travels in the feeder audit: {audit}"
    );
    assert_eq!(Some("Condition"), item["type"].as_str());
    assert_eq!(
        Some("ferrobridge.test"),
        audit["originating_system_audit"]["system_id"].as_str(),
        "the feeder audit names this bridge: {audit}"
    );
    Ok(())
}

#[tokio::test]
async fn a_derived_id_follows_the_r4_grammar() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    assert!(
        ferrobridge_server::facade::identity::is_fhir_id(&id),
        "{id} is outside [A-Za-z0-9\\-\\.]{{1,64}}"
    );
    Ok(())
}

#[tokio::test]
async fn the_same_composition_read_twice_yields_the_same_id() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let first = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    let second = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, first.0, "{:?}", first.1);
    assert_eq!(first.1["id"], second.1["id"]);
    assert_eq!(Some(id.as_str()), first.1["id"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_new_version_keeps_the_id_and_moves_the_version_id() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_TWO, &harness.composition()).await;
    let (status, body) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some(id.as_str()), body["id"].as_str());
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_read_of_an_id_the_map_does_not_know_is_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::NOT_FOUND, status);
    assert_eq!(Some("not-found"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_composition_the_cdr_reports_deleted_is_four_hundred_and_ten()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/composition/{CONTAINER}"
        )))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::GONE, status);
    assert_eq!(Some("deleted"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_re_sent_resource_updates_the_same_composition() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(harness.composition().to_string()),
    )
    .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(
        StatusCode::OK,
        status,
        "a re-sent resource updates rather than duplicates: {body}"
    );
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "the second send created no second composition"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_creates_when_nothing_matches() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", "_id=notrecordedyet")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    Ok(())
}

#[tokio::test]
async fn if_none_exist_answers_two_hundred_when_one_matches() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let mut fresh = condition();
    fresh["id"] = serde_json::json!("another-sender-id");
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", format!("_id={id}"))
        .body(Body::from(fresh.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some(id.as_str()), body["id"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "a conditional create that matched created nothing"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_answers_four_hundred_and_twelve_when_several_match()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let first = create_one(&harness).await?;
    let second = ferrobridge_server::facade::identity::FhirResourceId::new("secondmatch")?;
    harness.store.record_binding(
        "Condition",
        &second,
        &harness
            .store
            .binding_of(
                "Condition",
                &ferrobridge_server::facade::identity::FhirResourceId::new(&first)?,
            )?
            .expect("the first create recorded a binding"),
    )?;
    // A resource the map already consumed resolves to an update before any
    // conditional create runs, so this case sends a source id the map does
    // not know (`docs/architecture.md` §4.6).
    let mut fresh = condition();
    fresh["id"] = serde_json::json!("a-sender-id-the-map-has-not-seen");
    let request = Request::post("/fhir/Condition")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header("If-None-Exist", format!("_id={first},secondmatch"))
        .body(Body::from(fresh.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn an_update_of_an_id_the_map_does_not_know_is_four_hundred_and_four()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let request = Request::put("/fhir/Condition/abcdef")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(
        StatusCode::NOT_FOUND,
        status,
        "updateCreate is false, so a PUT to an unknown id creates nothing"
    );
    assert_eq!(Some("not-found"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn an_update_without_if_match_checks_the_current_etag() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(harness.composition().to_string()),
    )
    .await;
    let request = Request::put(format!("/fhir/Condition/{id}"))
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert!(
        harness
            .received("GET", &format!("/ehr/{EHR_ID}/composition/{CONTAINER}"))
            .await
            >= 1,
        "a PUT without If-Match reads the CDR's current ETag first"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_precondition_failure_is_four_hundred_and_twelve_with_the_current_etag()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(412).insert_header("ETag", format!("W/\"{VERSION_TWO}\"")),
    )
    .await;
    let request = Request::put(format!("/fhir/Condition/{id}"))
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .header(header::IF_MATCH, format!("W/\"{VERSION_ONE}\""))
        .body(Body::from(condition().to_string()))?;
    let response = raw(harness.app(), request).await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, response.status());
    assert_eq!(
        Some("W/\"2\""),
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        "the 412 carries the version the CDR reports as current"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_validation_refusal_is_four_hundred_and_twenty_two_with_its_errors()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(ferrobridge_testkit::stubs::its_rest::unprocessable(
            "the composition is invalid",
            &["/content[0] is required"],
        ))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .expect("the refusal carries the CDR's own words");
    assert!(diagnostics.contains("the composition is invalid"), "{body}");
    assert!(
        diagnostics.contains("/content[0] is required"),
        "the validationErrors travel verbatim: {body}"
    );
    assert!(
        !body.to_string().contains("\"validationErrors\""),
        "the openEHR error body never reaches the wire raw: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_five_hundred_is_five_hundred_and_two_carrying_the_upstream_status()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(ResponseTemplate::new(503))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::BAD_GATEWAY, status);
    assert!(
        first_issue(&body)["diagnostics"]
            .as_str()
            .is_some_and(|text| text.contains("503")),
        "the 502 names the upstream status: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_four_hundred_and_one_stays_four_hundred_and_one_with_its_challenge()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Bearer realm=\"cdr\""),
        )
        .mount(&harness.cdr)
        .await;
    let response = raw(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::UNAUTHORIZED, response.status());
    assert_ne!(
        StatusCode::FORBIDDEN,
        response.status(),
        "a CDR 401 never becomes a 403"
    );
    assert_eq!(
        Some("Bearer realm=\"cdr\""),
        response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
    );
    Ok(())
}

#[tokio::test]
async fn a_cdr_status_its_rest_does_not_document_is_five_hundred() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(ResponseTemplate::new(415))
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(
        StatusCode::INTERNAL_SERVER_ERROR,
        status,
        "a 415 from the CDR means the bridge chose a call it does not offer: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn validate_answers_two_hundred_and_commits_nothing() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let request = Request::post(VALIDATE)
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(
        Some("information"),
        first_issue(&body)["severity"].as_str(),
        "a resource that would commit reports information: {body}"
    );
    assert_eq!(
        0,
        harness.received("POST", &format!("/ehr/{EHR_ID}")).await,
        "the dry run committed something"
    );
    assert_eq!(
        0,
        harness.received("GET", "/ehr").await,
        "the dry run resolved an EHR it never writes into"
    );
    Ok(())
}

#[tokio::test]
async fn validate_reports_the_verdict_of_a_resource_that_would_not_commit()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut resource = condition();
    resource["subject"] = serde_json::json!({ "display": "no identifier and no reference" });
    let request = Request::post(VALIDATE)
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(resource.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "both verdicts answer 200: {body}");
    let issue = first_issue(&body);
    assert_eq!(Some("error"), issue["severity"].as_str(), "{body}");
    assert!(
        issue["details"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("documents no composition-validation route")),
        "the outcome says which validator ran: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn validate_mirrors_the_create_paths_operation_level_statuses()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut resource = condition();
    resource["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    let request = Request::post(VALIDATE)
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(resource.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(
        StatusCode::UNPROCESSABLE_ENTITY,
        status,
        "no program is the create path's 422 here too: {body}"
    );
    let (unsupported, _) = call(
        harness.app(),
        Request::post("/fhir/Observation/$validate")
            .header(header::CONTENT_TYPE, "application/fhir+json")
            .body(Body::from(condition().to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::NOT_FOUND, unsupported);
    Ok(())
}

#[tokio::test]
async fn a_transaction_with_one_failing_entry_commits_nothing_and_names_it()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let mut broken = condition();
    broken["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-good", "resource": condition(),
              "request": { "method": "POST", "url": "Condition" } },
            { "fullUrl": "urn:uuid:0000-bad", "resource": broken,
              "request": { "method": "POST", "url": "Condition" } }
        ]
    });
    let request = Request::post("/fhir")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(bundle.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    assert!(
        body.to_string().contains("urn:uuid:0000-bad"),
        "the refusal names the failing entry by its fullUrl: {body}"
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "a transaction that cannot be mapped in full commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_transaction_that_maps_in_full_commits_one_contribution() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", "W/\"7b0a4c2e-0000-4000-8000-00000000000c\""),
        )
        .mount(&harness.cdr)
        .await;
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            { "fullUrl": "urn:uuid:0000-good", "resource": condition(),
              "request": { "method": "POST", "url": "Condition" } }
        ]
    });
    let request = Request::post("/fhir")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(bundle.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the whole Bundle commits as one contribution"
    );
    Ok(())
}

#[tokio::test]
async fn a_batch_bundle_is_not_supported_in_this_milestone() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    // FHIR JSON forbids an empty array (<https://hl7.org/fhir/R4/json.html>),
    // so a batch Bundle with no entry omits the element rather than sending
    // one the strict codec would refuse before the batch check runs.
    let bundle = serde_json::json!({ "resourceType": "Bundle", "type": "batch" });
    let request = Request::post("/fhir")
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(bundle.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status);
    assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn every_answer_carries_the_fhir_media_type() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    for request in [
        Request::get("/fhir/metadata").body(Body::empty())?,
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
        Request::get("/fhir/Observation/abcdef").body(Body::empty())?,
    ] {
        let response = raw(harness.app(), request).await?;
        assert_eq!(
            Some("application/fhir+json"),
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            "{:?}",
            response.status()
        );
    }
    Ok(())
}
