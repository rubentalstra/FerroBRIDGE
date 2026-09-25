// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR facade on the wire, through the library run path.
//!
//! Every case drives the shipped router over a `wiremock` CDR, so the stack
//! under test is the stack the binary serves: the middleware, the media-type
//! guard, the program selection, the engine, the identity map and the status
//! table.

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
use std::time::Duration;
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

/// Returns the `201` a contribution commit answers under
/// `Prefer: return=representation`: the CONTRIBUTION, whose `versions`
/// reference each committed version (`ehr-codegen.openapi.yaml`,
/// `201_CONTRIBUTION` and the `Contribution` schema).
pub(crate) fn contribution_created(contribution: &str, versions: &[&str]) -> ResponseTemplate {
    let body = ferrobridge_testkit::stubs::its_rest::contribution_body(contribution, versions);
    ResponseTemplate::new(201)
        .insert_header("ETag", format!("W/\"{contribution}\""))
        .insert_header("Content-Type", "application/json")
        .set_body_string(body)
}

/// How the echo CDR answers what a contribution committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Echo {
    /// The version list in the order the versions were sent.
    Sent,
    /// An empty `201`, as a CDR that does not honour
    /// `Prefer: return=representation` answers, with the CONTRIBUTION served
    /// on its read.
    Minimal,
    /// The version list reversed, which ITS-REST does not forbid.
    Reversed,
    /// The versions in order, each read back naming an item no entry sent.
    Foreign,
}

/// The compositions a stub CDR committed, by version and by container.
#[derive(Debug, Default)]
struct Committed {
    /// Every committed composition, in commit order.
    versions: Vec<(String, serde_json::Value)>,
    /// How many version containers the commits created.
    created: usize,
    /// The versions the latest contribution committed, in commit order.
    latest: Vec<String>,
}

impl Committed {
    /// Returns the distinct version containers committed, in commit order.
    fn containers(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for (version, _) in &self.versions {
            let container = version.split("::").next().unwrap_or_default();
            if !seen.contains(&container) {
                seen.push(container);
            }
        }
        seen
    }
}

/// The stub CDR half that takes a contribution and remembers its versions.
struct TakeContribution {
    /// What was committed.
    committed: Arc<std::sync::Mutex<Committed>>,
    /// The version containers handed out, in order.
    containers: &'static [&'static str],
    /// How the answer lists the versions.
    echo: Echo,
    /// How long the answer waits after the commit is taken.
    delay: Duration,
}

impl wiremock::Respond for TakeContribution {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        self.answer(request).set_delay(self.delay)
    }
}

impl TakeContribution {
    /// Takes the contribution `request` carries and answers it.
    fn answer(&self, request: &wiremock::Request) -> ResponseTemplate {
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
            return ResponseTemplate::new(400);
        };
        let Ok(mut committed) = self.committed.lock() else {
            return ResponseTemplate::new(500);
        };
        let mut named = Vec::new();
        for data in body["versions"].as_array().into_iter().flatten() {
            // A version that names its `preceding_version_uid` is the next
            // version of that container (`ehr-codegen.openapi.yaml`,
            // `UpdateVersion`); any other opens the next container.
            let version = if let Some(prior) = data["preceding_version_uid"]["value"].as_str() {
                let mut parts = prior.split("::");
                let container = parts.next().unwrap_or_default();
                let tree = parts.nth(1).and_then(|tree| tree.parse::<u32>().ok());
                let Some(tree) = tree else {
                    return ResponseTemplate::new(400);
                };
                format!("{container}::ferrobridge.test::{}", tree + 1)
            } else {
                let Some(container) = self.containers.get(committed.created) else {
                    return ResponseTemplate::new(500);
                };
                committed.created += 1;
                format!("{container}::ferrobridge.test::1")
            };
            let mut stored = data["data"].clone();
            if self.echo == Echo::Foreign {
                stored["feeder_audit"]["originating_system_item_ids"][0]["id"] =
                    serde_json::json!("an-item-no-entry-sent");
            }
            committed.versions.push((version.clone(), stored));
            named.push(version);
        }
        committed.latest.clone_from(&named);
        if self.echo == Echo::Reversed {
            named.reverse();
        }
        if self.echo == Echo::Minimal {
            return ferrobridge_testkit::stubs::its_rest::contribution_created_minimal(
                CONTRIBUTION,
            );
        }
        let named: Vec<&str> = named.iter().map(String::as_str).collect();
        contribution_created(CONTRIBUTION, &named)
    }
}

/// The stub CDR half that answers the latest CONTRIBUTION it committed on its
/// read (`ehr-codegen.openapi.yaml`, `contribution_get`).
struct ReadContribution {
    /// What was committed.
    committed: Arc<std::sync::Mutex<Committed>>,
}

impl wiremock::Respond for ReadContribution {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let Ok(committed) = self.committed.lock() else {
            return ResponseTemplate::new(500);
        };
        let named: Vec<&str> = committed.latest.iter().map(String::as_str).collect();
        if named.is_empty() {
            return ferrobridge_testkit::stubs::its_rest::not_found(
                "no contribution was committed",
            );
        }
        ferrobridge_testkit::stubs::its_rest::retrieved(
            &ferrobridge_testkit::stubs::its_rest::contribution_body(CONTRIBUTION, &named),
        )
    }
}

/// The stub CDR half that answers a committed composition by its version, or
/// by its container at the latest version.
struct ReadCommitted {
    /// What was committed.
    committed: Arc<std::sync::Mutex<Committed>>,
}

impl wiremock::Respond for ReadCommitted {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let Ok(committed) = self.committed.lock() else {
            return ResponseTemplate::new(500);
        };
        let wanted = request
            .url
            .path_segments()
            .and_then(Iterator::last)
            .map(|segment| segment.replace("%3A", ":").replace("%3a", ":"))
            .unwrap_or_default();
        let found = committed.versions.iter().rev().find(|(version, _)| {
            *version == wanted || version.split("::").next() == Some(wanted.as_str())
        });
        match found {
            Some((version, composition)) => ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{version}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(composition.to_string()),
            None => ResponseTemplate::new(404),
        }
    }
}

/// Mounts a stub CDR that takes contributions, handing out `containers` in
/// order, and reads back what it committed.
pub(crate) async fn mount_echo(cdr: &MockServer, containers: &'static [&'static str], echo: Echo) {
    mount_echo_after(cdr, containers, echo, Duration::ZERO).await;
}

/// Mounts the echo CDR of [`mount_echo`], whose contribution answer waits
/// `delay` after the commit is taken.
async fn mount_echo_after(
    cdr: &MockServer,
    containers: &'static [&'static str],
    echo: Echo,
    delay: Duration,
) {
    mount_echo_tracked(cdr, containers, echo, delay).await;
}

/// Mounts the echo CDR of [`mount_echo_after`], and returns what it commits
/// so a case can count the containers.
async fn mount_echo_tracked(
    cdr: &MockServer,
    containers: &'static [&'static str],
    echo: Echo,
    delay: Duration,
) -> Arc<std::sync::Mutex<Committed>> {
    let committed = Arc::new(std::sync::Mutex::new(Committed::default()));
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(TakeContribution {
            committed: Arc::clone(&committed),
            containers,
            echo,
            delay,
        })
        .mount(cdr)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ReadContribution {
            committed: Arc::clone(&committed),
        })
        .mount(cdr)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path_regex(format!(
            "^/ehr/{EHR_ID}/composition/.+$"
        )))
        .respond_with(ReadCommitted {
            committed: Arc::clone(&committed),
        })
        .mount(cdr)
        .await;
    committed
}

/// The containers the echo CDR hands out for a transaction.
const ECHOED: &[&str] = &[
    CONTAINER,
    "5f0e2b1a-0000-4000-8000-00000000000d",
    "6a1f3c2b-0000-4000-8000-00000000000f",
];

/// The containers the echo CDR hands out beside a create that took
/// [`CONTAINER`].
const ECHOED_BESIDE_CREATE: &[&str] = &["5f0e2b1a-0000-4000-8000-00000000000d"];

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

/// Returns the `(name, definition)` pairs of the system-level operations
/// `body` declares.
fn system_operations(body: &serde_json::Value) -> Vec<(String, String)> {
    body["rest"][0]["operation"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    (
                        entry["name"].as_str().unwrap_or_default().to_owned(),
                        entry["definition"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_template_the_cdr_does_not_hold_names_the_context_and_the_status()
-> Result<(), Box<dyn StdError>> {
    // An unmatched route answers 404 on both definition routes of ITS-REST.
    let cdr = MockServer::start().await;
    let set = programs::read_set(std::path::Path::new(FIXTURES))?;
    let error = programs::fetch_templates(&set, &client(&cdr))
        .await
        .err()
        .ok_or("a template the CDR does not hold refuses the load")?;
    match error {
        programs::LoadError::UnknownTemplate {
            ref template,
            ref context,
            status,
        } => {
            assert_eq!("ferrobridge.diagnose.v1", template);
            assert_eq!("ferrobridge_facade.context", context);
            assert_eq!(StatusCode::NOT_FOUND, status);
        }
        ref other => panic!("the refusal names the context and the status: {other}"),
    }
    Ok(())
}

#[tokio::test]
async fn a_served_operations_lane_is_declared_at_the_system_level() -> Result<(), Box<dyn StdError>>
{
    // R4 CapabilityStatement.rest.operation carries the system-level
    // operations; both FSH definitions of the draft chapter set `system = true`.
    let harness = harness().await;
    let (status, body) = call(
        harness.app_with_operations(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(
        vec![
            (
                String::from("tofhir"),
                String::from("http://fhirconnect.org/fhir/OperationDefinition/ToFhir"),
            ),
            (
                String::from("toopenehr"),
                String::from("http://fhirconnect.org/fhir/OperationDefinition/ToOpenEhr"),
            ),
        ],
        system_operations(&body)
    );
    Ok(())
}

#[tokio::test]
async fn an_absent_operations_lane_is_not_declared() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert!(
        body["rest"][0]["operation"].is_null(),
        "an operations lane that is not served is not declared: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn the_direct_forms_of_the_operations_are_never_declared() -> Result<(), Box<dyn StdError>> {
    // rest-api.adoc (draft), Direct payload invocation: "intentionally not
    // part of the FHIRconnect FHIR Implementation Guide".
    let harness = harness().await;
    let (_, body) = call(
        harness.app_with_operations(),
        Request::get("/fhir/metadata").body(Body::empty())?,
    )
    .await?;
    let rendered = body.to_string();
    assert!(
        !rendered.contains("/tofhir") && !rendered.contains("/toopenehr"),
        "a direct form is declared: {rendered}"
    );
    let names: Vec<String> = system_operations(&body)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        vec![String::from("tofhir"), String::from("toopenehr")],
        names
    );
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
    // A re-sent body with no `meta.versionId` and the content the map already
    // holds is a replay, so this case re-sends the resource with its content
    // changed, which is the update.
    let (status, body) = call(harness.app(), post_condition(&revised_condition())).await?;
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
    assert_eq!(
        1,
        harness
            .received("PUT", &format!("/ehr/{EHR_ID}/composition/{CONTAINER}"))
            .await,
        "the second send committed one later version"
    );
    Ok(())
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

/// Mounts the composition update that answers `200` with [`VERSION_TWO`].
async fn mount_version_two(harness: &Harness) {
    mount_update(
        &harness.cdr,
        ResponseTemplate::new(200)
            .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(harness.composition().to_string()),
    )
    .await;
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

#[tokio::test]
async fn a_create_re_sent_with_its_id_and_version_id_commits_nothing_and_names_the_first_composition()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, first_location, first) = post_located(&harness, &condition_at("1")).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, location, body) = post_located(&harness, &condition_at("1")).await?;
    assert_eq!(StatusCode::OK, status, "a replay answers 200 OK: {body}");
    assert!(first_location.is_some(), "{first}");
    assert_eq!(
        first_location, location,
        "the replay names the composition the first delivery produced"
    );
    assert_eq!(first["id"], body["id"]);
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert_eq!(
        (1, 0, 0),
        writes(&harness).await,
        "one composition POST across both deliveries, and no update or contribution"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_of_a_known_id_at_a_new_version_id_commits_a_later_version()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, _, first) = post_located(&harness, &condition_at("1")).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, location, body) = post_located(&harness, &condition_at("2")).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(first["id"], body["id"], "the same resource, one version on");
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    let id = first["id"]
        .as_str()
        .ok_or("the created resource carries an id")?;
    assert_eq!(
        Some(format!("{BASE_URL}/Condition/{id}/_history/2")),
        location
    );
    assert_eq!(
        (1, 1, 0),
        writes(&harness).await,
        "one composition POST and one update PUT"
    );
    let source = ferrobridge_server::facade::identity::record::SourceVersion::new(
        "Condition",
        ferrobridge_server::facade::identity::ExternalResourceId::new(
            "ferrobridge-synthetic-condition-1",
        )?,
        Some(String::from("2")),
    );
    assert_eq!(
        Some(CONTAINER),
        harness
            .store
            .consumed(&source)?
            .as_ref()
            .map(|known| known.versioned_object_uid.as_str()),
        "the new version is recorded against the same composition"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_re_sent_without_a_version_id_and_with_the_same_content_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, first_location, first) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, location, body) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(first_location, location);
    assert_eq!(Some("1"), body["meta"]["versionId"].as_str());
    assert_eq!(
        (1, 0, 0),
        writes(&harness).await,
        "the same content with no versionId is a replay"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_re_sent_without_a_version_id_and_with_other_content_commits_a_later_version()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    mount_version_two(&harness).await;
    let (first_status, _, first) = post_located(&harness, &condition()).await?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    let (status, _, body) = post_located(&harness, &revised_condition()).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(first["id"], body["id"]);
    assert_eq!(Some("2"), body["meta"]["versionId"].as_str());
    assert_eq!(
        (1, 1, 0),
        writes(&harness).await,
        "other content with no versionId is the update"
    );
    Ok(())
}

#[tokio::test]
async fn a_create_of_an_id_the_map_binds_to_two_compositions_is_refused_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let source = |version: &str| -> Result<_, Box<dyn StdError>> {
        Ok(
            ferrobridge_server::facade::identity::record::SourceVersion::new(
                "Condition",
                ferrobridge_server::facade::identity::ExternalResourceId::new(
                    "ferrobridge-synthetic-condition-1",
                )?,
                Some(String::from(version)),
            ),
        )
    };
    for (version, container) in [
        ("1", CONTAINER),
        ("2", "5f0e2b1a-0000-4000-8000-00000000000d"),
    ] {
        harness.store.record_consumed(
            &source(version)?,
            &ferrobridge_server::facade::identity::record::ConsumedSource {
                ehr_id: String::from(EHR_ID),
                versioned_object_uid: String::from(container),
                internal_id: String::from("abc"),
                context: String::from("ferrobridge_facade.context"),
            },
        )?;
    }
    let (status, _, body) = post_located(&harness, &condition_at("3")).await?;
    assert_eq!(StatusCode::CONFLICT, status, "{body}");
    assert_eq!(Some("conflict"), first_issue(&body)["code"].as_str());
    assert_eq!((0, 0, 0), writes(&harness).await, "nothing is committed");
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
    // not know.
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

/// The `Identifier.system` of the synthetic Condition's one identifier.
const CONDITION_IDENTIFIER_SYSTEM: &str = "http://example.org/fhir/sid/ferrobridge-condition";

/// The `Identifier.value` of the synthetic Condition's one identifier.
const CONDITION_IDENTIFIER_VALUE: &str = "synthetic-condition-0001";

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

#[tokio::test]
async fn if_none_exist_on_an_identifier_a_committed_resource_carries_answers_two_hundred_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    // "If the search finds one match, the server returns 200 OK"
    // (<https://hl7.org/fhir/R4/http.html#ccreate>); the token forms are
    // `[system]|[code]` and `[code]` (<https://hl7.org/fhir/R4/search.html#token>).
    let harness = harness().await;
    let id = create_one(&harness).await?;
    mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    for search in [
        format!("identifier={CONDITION_IDENTIFIER_SYSTEM}|{CONDITION_IDENTIFIER_VALUE}"),
        format!("identifier={CONDITION_IDENTIFIER_VALUE}"),
        format!(
            "identifier=http%3A%2F%2Fexample.org%2Ffhir%2Fsid%2Fferrobridge-condition%7C{CONDITION_IDENTIFIER_VALUE}"
        ),
    ] {
        let (status, body) = call(harness.app(), conditional_create(&search)?).await?;
        assert_eq!(StatusCode::OK, status, "{search}: {body}");
        assert_eq!(Some(id.as_str()), body["id"].as_str(), "{search}");
    }
    assert_eq!(
        (1, 0, 0),
        writes(&harness).await,
        "the one create before the searches is the only commit"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_on_an_identifier_no_resource_carries_still_creates()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    create_one(&harness).await?;
    for (sender_id, search) in [
        (
            "sender-a",
            format!("identifier={CONDITION_IDENTIFIER_SYSTEM}|another-value"),
        ),
        (
            "sender-b",
            format!("identifier=http://example.org/another-system|{CONDITION_IDENTIFIER_VALUE}"),
        ),
        (
            "sender-c",
            format!("identifier=|{CONDITION_IDENTIFIER_VALUE}"),
        ),
    ] {
        let (status, body) =
            call(harness.app(), conditional_create_as(sender_id, &search)?).await?;
        assert_eq!(StatusCode::CREATED, status, "{search}: {body}");
    }
    assert_eq!(
        (4, 0, 0),
        writes(&harness).await,
        "each unmatched conditional create commits its composition"
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_on_an_identifier_two_resources_carry_is_four_hundred_and_twelve()
-> Result<(), Box<dyn StdError>> {
    // "If it finds more than one match, the server returns 412 Precondition
    // Failed" (<https://hl7.org/fhir/R4/http.html#ccreate>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Sent).await;
    let mut second = condition();
    second["id"] = serde_json::json!("ferrobridge-synthetic-condition-2");
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-first", condition()),
            ("urn:uuid:0000-second", second),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let (status, body) = call(
        harness.app(),
        conditional_create(&format!(
            "identifier={CONDITION_IDENTIFIER_SYSTEM}|{CONDITION_IDENTIFIER_VALUE}"
        ))?,
    )
    .await?;
    assert_eq!(StatusCode::PRECONDITION_FAILED, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the refused conditional create commits nothing"
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await
    );
    Ok(())
}

#[tokio::test]
async fn if_none_exist_on_every_code_of_a_system_is_refused() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let (status, body) = call(
        harness.app(),
        conditional_create(&format!("identifier={CONDITION_IDENTIFIER_SYSTEM}|"))?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status, "{body}");
    assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
    assert_eq!((0, 0, 0), writes(&harness).await);
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
    mount_echo(&harness.cdr, ECHOED, Echo::Sent).await;
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

/// Mounts the echo CDR whose first commit produces [`VERSION_ONE`].
async fn mount_contribution(cdr: &MockServer) {
    mount_echo(cdr, ECHOED, Echo::Sent).await;
}

#[tokio::test]
async fn each_transaction_entry_answers_the_location_and_etag_a_create_answers()
-> Result<(), Box<dyn StdError>> {
    // A transaction answers a `transaction-response` Bundle whose entries each
    // carry the status, `location` and `ETag`
    // (<https://hl7.org/fhir/R4/http.html#transaction-response>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(Some("Bundle"), body["resourceType"].as_str(), "{body}");
    assert_eq!(Some("transaction-response"), body["type"].as_str());
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("201 Created"), response["status"].as_str(), "{body}");
    assert_eq!(Some("W/\"1\""), response["etag"].as_str());
    let location = response["location"]
        .as_str()
        .ok_or("the entry answers a location")?;
    let id = location
        .strip_prefix(&format!("{BASE_URL}/Condition/"))
        .and_then(|rest| rest.strip_suffix("/_history/1"))
        .ok_or(format!("{location} is no [base]/Condition/[id]/_history/1"))?;
    let internal = ferrobridge_server::facade::identity::FhirResourceId::new(id)?;
    let binding = harness
        .store
        .binding_of("Condition", &internal)?
        .ok_or("the transaction recorded the entry's identity binding")?;
    assert_eq!(CONTAINER, binding.versioned_object_uid);
    assert_eq!(EHR_ID, binding.ehr_id);
    Ok(())
}

#[tokio::test]
async fn a_transaction_sent_twice_commits_once_and_answers_the_same_ids()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition())]);
    let (first_status, first) = post_transaction(&harness, &bundle).await?;
    let (second_status, second) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    assert_eq!(StatusCode::OK, second_status, "{second}");
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "a re-sent transaction commits nothing"
    );
    let first_location = first["entry"][0]["response"]["location"].as_str();
    assert!(first_location.is_some(), "{first}");
    assert_eq!(
        first_location,
        second["entry"][0]["response"]["location"].as_str(),
        "the re-sent transaction names the resource its first delivery created: {second}"
    );
    assert_eq!(
        Some("200 OK"),
        second["entry"][0]["response"]["status"].as_str()
    );
    Ok(())
}

/// How long the stub CDR holds a commit answer in the overlap cases.
const HELD: Duration = Duration::from_millis(800);

/// How long the second delivery of an overlap case waits before it starts,
/// well inside [`HELD`].
const STAGGER: Duration = Duration::from_millis(150);

/// Returns the `location` of every issue of an `OperationOutcome` body.
fn issue_locations(body: &serde_json::Value) -> Vec<&str> {
    body["issue"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|issue| issue["location"][0].as_str())
        .collect()
}

#[tokio::test]
async fn two_overlapping_deliveries_of_one_transaction_commit_once() -> Result<(), Box<dyn StdError>>
{
    // No specification governs the redelivery rule: our own design. The CDR
    // holds the first commit's answer, so the second delivery starts while the
    // first holds its claim, and is refused with nothing committed.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo_after(&harness.cdr, ECHOED, Echo::Sent, HELD).await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition())]);
    let (first, second) = tokio::join!(post_transaction(&harness, &bundle), async {
        tokio::time::sleep(STAGGER).await;
        post_transaction(&harness, &bundle).await
    });
    let (first_status, first) = first?;
    let (second_status, second) = second?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    assert_eq!(
        Some("201 Created"),
        first["entry"][0]["response"]["status"].as_str(),
        "{first}"
    );
    assert_eq!(StatusCode::CONFLICT, second_status, "{second}");
    assert_eq!(Some("duplicate"), first_issue(&second)["code"].as_str());
    assert_eq!(
        vec!["urn:uuid:0000-good"],
        issue_locations(&second),
        "{second}"
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the overlapping delivery commits nothing"
    );

    let (third_status, third) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, third_status, "{third}");
    assert_eq!(
        Some("200 OK"),
        third["entry"][0]["response"]["status"].as_str(),
        "sent after the first delivery settled, the Bundle answers as re-sent: {third}"
    );
    assert_eq!(
        first["entry"][0]["response"]["location"].as_str(),
        third["entry"][0]["response"]["location"].as_str(),
        "{third}"
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the delivery after the first settled commits nothing either"
    );
    Ok(())
}

#[tokio::test]
async fn two_overlapping_creates_of_one_resource_commit_once() -> Result<(), Box<dyn StdError>> {
    // No specification governs the redelivery rule: our own design. The CDR
    // holds the first create's answer while the second create of the same
    // `id` and `meta.versionId` arrives.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("ETag", format!("W/\"{VERSION_ONE}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(harness.composition().to_string())
                .set_delay(HELD),
        )
        .mount(&harness.cdr)
        .await;
    let (first, second) = tokio::join!(call(harness.app(), post_condition(&condition())), async {
        tokio::time::sleep(STAGGER).await;
        call(harness.app(), post_condition(&condition())).await
    });
    let (first_status, first) = first?;
    let (second_status, second) = second?;
    assert_eq!(StatusCode::CREATED, first_status, "{first}");
    assert_eq!(StatusCode::CONFLICT, second_status, "{second}");
    assert_eq!(Some("duplicate"), first_issue(&second)["code"].as_str());
    assert_eq!(
        vec!["Condition/ferrobridge-synthetic-condition-1"],
        issue_locations(&second),
        "{second}"
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "the overlapping create commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_partly_consumed_transaction_is_refused_naming_the_consumed_entries()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-old", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let mut fresh = condition();
    fresh["id"] = serde_json::json!("ferrobridge-synthetic-condition-2");
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-old", condition()),
            ("urn:uuid:0000-new", fresh),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::CONFLICT, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    let named: Vec<&str> = body["issue"]
        .as_array()
        .ok_or("the refusal carries issues")?
        .iter()
        .filter_map(|issue| issue["location"][0].as_str())
        .collect();
    assert_eq!(vec!["urn:uuid:0000-old"], named, "{body}");
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "a refused transaction commits nothing"
    );
    Ok(())
}

#[tokio::test]
async fn a_transaction_carrying_one_resource_twice_is_refused() -> Result<(), Box<dyn StdError>> {
    // "A resource can only appear in a transaction once (by identity)"
    // (<https://hl7.org/fhir/R4/http.html#transaction>).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_contribution(&harness.cdr).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-one", condition()),
            ("urn:uuid:0000-two", condition()),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    assert_eq!(Some("duplicate"), first_issue(&body)["code"].as_str());
    assert_eq!(
        Some("urn:uuid:0000-two"),
        first_issue(&body)["location"][0].as_str()
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await
    );
    Ok(())
}

#[tokio::test]
async fn a_contribution_answer_naming_no_versions_for_the_entries_is_a_typed_failure()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/contribution")))
        .respond_with(contribution_created(
            CONTRIBUTION,
            &[VERSION_ONE, VERSION_TWO],
        ))
        .mount(&harness.cdr)
        .await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .unwrap_or_default();
    assert!(
        diagnostics.contains(CONTRIBUTION),
        "the failure names the committed contribution: {body}"
    );
    Ok(())
}

/// Returns the container the identity map bound the entry `location` names
/// to.
fn bound_container(harness: &Harness, location: Option<&str>) -> Result<String, Box<dyn StdError>> {
    let location = location.ok_or("the entry answers a location")?;
    let id = location
        .strip_prefix(&format!("{BASE_URL}/Condition/"))
        .and_then(|rest| rest.split('/').next())
        .ok_or(format!("{location} is no Condition location"))?;
    let internal = ferrobridge_server::facade::identity::FhirResourceId::new(id)?;
    let binding = harness
        .store
        .binding_of("Condition", &internal)?
        .ok_or("the entry's binding is recorded")?;
    Ok(binding.versioned_object_uid)
}

#[tokio::test]
async fn a_reversed_version_list_still_binds_each_entry_to_its_own_composition()
-> Result<(), Box<dyn StdError>> {
    // ITS-REST 1.1.0 states no order for `CONTRIBUTION.versions`, so each
    // version is matched to its entry by the FEEDER_AUDIT read back.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Reversed).await;
    let mut second = condition();
    second["id"] = serde_json::json!("ferrobridge-synthetic-condition-2");
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[
            ("urn:uuid:0000-first", condition()),
            ("urn:uuid:0000-second", second),
        ]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let first_bound = bound_container(&harness, body["entry"][0]["response"]["location"].as_str())?;
    let second_bound =
        bound_container(&harness, body["entry"][1]["response"]["location"].as_str())?;
    let echoed_first = ECHOED.first().copied().ok_or("a first container")?;
    let echoed_second = ECHOED.get(1).copied().ok_or("a second container")?;
    assert_eq!(
        echoed_first, first_bound,
        "the first entry's own composition"
    );
    assert_eq!(
        echoed_second, second_bound,
        "the second entry's own composition"
    );
    Ok(())
}

#[tokio::test]
async fn a_committed_version_that_matches_no_entry_binds_nothing() -> Result<(), Box<dyn StdError>>
{
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Foreign).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .unwrap_or_default();
    assert!(
        diagnostics.contains(CONTRIBUTION) && diagnostics.contains("matches 0"),
        "the failure names the contribution and the unmatched version: {body}"
    );
    let source = ferrobridge_server::facade::identity::record::SourceVersion::new(
        "Condition",
        ferrobridge_server::facade::identity::ExternalResourceId::new(
            "ferrobridge-synthetic-condition-1",
        )?,
        None,
    );
    assert_eq!(
        None,
        harness.store.consumed(&source)?,
        "an unverified binding records no consumed source"
    );
    Ok(())
}

/// Returns the consumed-source key of the synthetic Condition.
fn condition_source()
-> Result<ferrobridge_server::facade::identity::record::SourceVersion, Box<dyn StdError>> {
    Ok(
        ferrobridge_server::facade::identity::record::SourceVersion::new(
            "Condition",
            ferrobridge_server::facade::identity::ExternalResourceId::new(
                "ferrobridge-synthetic-condition-1",
            )?,
            None,
        ),
    )
}

#[tokio::test]
async fn a_commit_answered_without_its_versions_binds_from_the_contribution_read_back()
-> Result<(), Box<dyn StdError>> {
    // An empty `201` names the contribution in its `ETag`
    // (`ehr-codegen.openapi.yaml`, `201_CONTRIBUTION`), and `contribution_get`
    // answers the CONTRIBUTION whose `versions` the entries bind from.
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_echo(&harness.cdr, ECHOED, Echo::Minimal).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("201 Created"), response["status"].as_str(), "{body}");
    assert_eq!(Some("W/\"1\""), response["etag"].as_str());
    assert_eq!(
        CONTAINER,
        bound_container(&harness, response["location"].as_str())?
    );
    assert_eq!(
        1,
        harness
            .received("GET", &format!("/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"))
            .await,
        "the versions are read back once"
    );
    assert!(harness.store.consumed(&condition_source()?)?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_contribution_that_cannot_be_read_back_is_a_typed_failure_and_its_retry_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "the contribution is not readable yet",
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.cdr)
        .await;
    mount_echo(&harness.cdr, ECHOED, Echo::Minimal).await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition())]);

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let diagnostics = first_issue(&body)["diagnostics"]
        .as_str()
        .unwrap_or_default();
    assert!(
        diagnostics.contains(CONTRIBUTION),
        "the failure names the committed contribution: {body}"
    );
    let source = condition_source()?;
    assert_eq!(None, harness.store.consumed(&source)?);
    let committed = harness
        .store
        .committed(&source)?
        .ok_or("the commit is recorded before the binding")?;
    assert_eq!(CONTRIBUTION, committed.contribution_uid);

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("200 OK"), response["status"].as_str(), "{body}");
    assert_eq!(
        CONTAINER,
        bound_container(&harness, response["location"].as_str())?
    );
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the retry commits nothing"
    );
    assert!(harness.store.consumed(&source)?.is_some());
    Ok(())
}

#[tokio::test]
async fn a_single_create_of_a_committed_unbound_entry_binds_from_the_read_back_and_commits_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "the contribution is not readable yet",
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.cdr)
        .await;
    mount_echo(&harness.cdr, ECHOED, Echo::Minimal).await;
    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition())]),
    )
    .await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let source = condition_source()?;
    assert_eq!(None, harness.store.consumed(&source)?);
    assert!(harness.store.committed(&source)?.is_some());

    let response = raw(harness.app(), post_condition(&condition())).await?;
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    assert_eq!(
        StatusCode::OK,
        status,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(CONTAINER, bound_container(&harness, location.as_deref())?);
    let rendered: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(Some("Condition"), rendered["resourceType"].as_str());
    assert_eq!(
        1,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "one contribution across both deliveries"
    );
    assert_eq!(
        0,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/composition"))
            .await,
        "the single create commits nothing"
    );
    assert!(harness.store.consumed(&source)?.is_some());
    Ok(())
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

/// Returns how many version containers the echo CDR committed into.
fn containers_of(committed: &Arc<std::sync::Mutex<Committed>>) -> Result<usize, Box<dyn StdError>> {
    let held = committed
        .lock()
        .map_err(|_poisoned| "the echo CDR's record is poisoned")?;
    Ok(held.containers().len())
}

#[tokio::test]
async fn a_transaction_entry_at_a_new_version_id_of_a_known_resource_commits_one_modification()
-> Result<(), Box<dyn StdError>> {
    // A CONTRIBUTION version that follows another names it as its
    // `preceding_version_uid` (`ehr-codegen.openapi.yaml`, `UpdateVersion`),
    // and its audit states a modification (openEHR terminology code 251).
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let committed = mount_echo_tracked(&harness.cdr, ECHOED, Echo::Sent, Duration::ZERO).await;
    let (first_status, first) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition_at("1"))]),
    )
    .await?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    let first_location = first["entry"][0]["response"]["location"]
        .as_str()
        .ok_or("the first delivery answers a location")?;
    let id = first_location
        .strip_prefix(&format!("{BASE_URL}/Condition/"))
        .and_then(|rest| rest.strip_suffix("/_history/1"))
        .ok_or(format!("{first_location} is no first version"))?
        .to_owned();

    let (status, body) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition_at("2"))]),
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("200 OK"), response["status"].as_str(), "{body}");
    assert_eq!(
        Some(format!("{BASE_URL}/Condition/{id}/_history/2").as_str()),
        response["location"].as_str(),
        "the same resource, one version on"
    );
    assert_eq!(Some("W/\"2\""), response["etag"].as_str());
    assert_eq!(
        2,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "one contribution per delivery"
    );
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/contribution"))
        .await
        .ok_or("the second contribution was sent")?;
    let versions = sent["versions"]
        .as_array()
        .ok_or("the contribution carries versions")?;
    assert_eq!(1, versions.len(), "one modification: {sent}");
    let version = versions.first().ok_or("one version")?;
    assert_eq!(
        Some(VERSION_ONE),
        version["preceding_version_uid"]["value"].as_str(),
        "the modification follows the version the CDR holds: {sent}"
    );
    assert_eq!(
        Some("251"),
        version["commit_audit"]["change_type"]["defining_code"]["code_string"].as_str(),
        "{sent}"
    );
    assert_eq!(1, containers_of(&committed)?, "no second composition");
    assert_eq!(
        Some(CONTAINER),
        harness
            .store
            .consumed(&condition_source_at("2")?)?
            .as_ref()
            .map(|known| known.versioned_object_uid.as_str()),
        "the new version is recorded against the same composition"
    );
    let (status, read) = call(
        harness.app(),
        Request::get(format!("/fhir/Condition/{id}")).body(Body::empty())?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{read}");
    assert_eq!(Some(id.as_str()), read["id"].as_str());
    assert_eq!(Some("2"), read["meta"]["versionId"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_revising_transaction_entry_whose_binding_failed_binds_on_retry_and_commits_once()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    let committed = mount_echo_tracked(&harness.cdr, ECHOED, Echo::Minimal, Duration::ZERO).await;
    let (first_status, first) = post_transaction(
        &harness,
        &transaction_of(&[("urn:uuid:0000-good", condition_at("1"))]),
    )
    .await?;
    assert_eq!(StatusCode::OK, first_status, "{first}");
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "/ehr/{EHR_ID}/contribution/{CONTRIBUTION}"
        )))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "the contribution is not readable yet",
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.cdr)
        .await;
    let bundle = transaction_of(&[("urn:uuid:0000-good", condition_at("2"))]);

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    let source = condition_source_at("2")?;
    assert_eq!(None, harness.store.consumed(&source)?);
    assert!(
        harness.store.committed(&source)?.is_some(),
        "the commit is recorded before the binding"
    );

    let (status, body) = post_transaction(&harness, &bundle).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let response = &body["entry"][0]["response"];
    assert_eq!(Some("200 OK"), response["status"].as_str(), "{body}");
    assert_eq!(Some("W/\"2\""), response["etag"].as_str(), "{body}");
    assert_eq!(
        CONTAINER,
        bound_container(&harness, response["location"].as_str())?
    );
    assert_eq!(
        2,
        harness
            .received("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await,
        "the retry commits nothing"
    );
    assert_eq!(1, containers_of(&committed)?, "no second composition");
    assert_eq!(
        Some(CONTAINER),
        harness
            .store
            .consumed(&source)?
            .as_ref()
            .map(|known| known.versioned_object_uid.as_str())
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

/// Returns `error` and every cause behind it as one line, the way the facade
/// renders a refusal into `issue.diagnostics`.
fn chain_of(error: &dyn StdError) -> String {
    let mut line = error.to_string();
    let mut cause = error.source();
    while let Some(source) = cause {
        line.push_str(": ");
        line.push_str(&source.to_string());
        cause = source.source();
    }
    line
}

/// Returns the synthetic Condition with no diagnosis name, which the
/// synthetic template requires.
fn condition_without_code() -> serde_json::Value {
    let mut resource = condition();
    if let Some(object) = resource.as_object_mut() {
        object.remove("code");
    }
    resource
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

#[tokio::test]
async fn validate_names_the_template_and_the_ehr_disposition() -> Result<(), Box<dyn StdError>> {
    // The carry-over case of #115: `information` issues naming what would be
    // committed and where.
    let harness = harness().await;
    let request = Request::post(VALIDATE)
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let information: Vec<&serde_json::Value> = body["issue"]
        .as_array()
        .ok_or("the outcome carries issues")?
        .iter()
        .filter(|issue| issue["severity"].as_str() == Some("information"))
        .collect();
    assert!(
        information.iter().any(|issue| issue["diagnostics"]
            .as_str()
            .is_some_and(|text| text.contains("ferrobridge.diagnose.v1"))),
        "an information issue names the template: {body}"
    );
    assert!(
        information.iter().any(|issue| issue["diagnostics"]
            .as_str()
            .is_some_and(|text| text.contains("synthetic-subject-0001")
                && text.contains("writes into an existing EHR only"))),
        "an information issue names the subject and the EHR policy: {body}"
    );
    assert_nothing_written(&harness).await;
    assert_eq!(
        0,
        harness.received("GET", "/ehr").await,
        "the dry run asked the CDR for an EHR"
    );
    Ok(())
}

#[tokio::test]
async fn validate_names_the_ehr_the_identity_map_already_holds() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    create_one(&harness).await?;
    let before = harness.received("POST", "/ehr").await;
    let request = Request::post(VALIDATE)
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(condition().to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert!(
        diagnostics_of(&body)
            .iter()
            .any(|text| text.contains(EHR_ID)),
        "the disposition names the EHR the map holds: {body}"
    );
    assert_eq!(
        before,
        harness.received("POST", "/ehr").await,
        "the dry run wrote into the CDR"
    );
    Ok(())
}

#[tokio::test]
async fn validate_carries_the_validators_message_verbatim_and_writes_nothing()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let resource = condition_without_code();
    let loaded = harness
        .facade
        .programs()
        .by_context("ferrobridge_facade.context")
        .ok_or("the fixture context compiled")?;
    let document: fhir_types::codec::Value = serde_json::from_value(resource.clone())?;
    let refusal = ferrobridge_server::facade::engine::inbound(
        loaded.program(),
        loaded.index(),
        &document,
        "2026-09-15T09:00:00Z",
        &settings(),
    )
    .err()
    .ok_or("the synthetic template requires the diagnosis name")?;
    let request = Request::post(VALIDATE)
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(resource.to_string()))?;
    let (status, body) = call(harness.app(), request).await?;
    assert_eq!(StatusCode::OK, status, "both verdicts answer 200: {body}");
    let issue = first_issue(&body);
    assert_eq!(Some("error"), issue["severity"].as_str(), "{body}");
    assert_eq!(
        Some(chain_of(&refusal).as_str()),
        issue["diagnostics"].as_str(),
        "the validator's message travels verbatim: {body}"
    );
    assert_nothing_written(&harness).await;
    assert_eq!(0, harness.received("GET", "/ehr").await);
    Ok(())
}

#[tokio::test]
async fn validate_refuses_at_the_operation_level_as_create_does() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    let mut unknown_member = condition();
    unknown_member["notAnElement"] = serde_json::json!("x");
    let mut other_profile = condition();
    other_profile["meta"] = serde_json::json!({ "profile": ["http://example.org/other"] });
    for (media, body) in [
        ("application/fhir+xml", String::from("<Condition/>")),
        ("application/fhir+json", unknown_member.to_string()),
        ("application/fhir+json", other_profile.to_string()),
        ("application/fhir+json", String::from("{")),
    ] {
        let created = call(
            harness.app(),
            Request::post("/fhir/Condition")
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(body.clone()))?,
        )
        .await?;
        let validated = call(
            harness.app(),
            Request::post(VALIDATE)
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(body.clone()))?,
        )
        .await?;
        assert_eq!(
            created.0, validated.0,
            "create and $validate answer the same status for {body}"
        );
        assert_eq!(
            first_issue(&created.1)["code"],
            first_issue(&validated.1)["code"],
            "create and $validate refuse with the same issue code for {body}"
        );
    }
    assert_nothing_written(&harness).await;
    Ok(())
}

#[tokio::test]
async fn invalid_mapped_content_stores_nothing() -> Result<(), Box<dyn StdError>> {
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
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    let source = ferrobridge_server::facade::identity::ExternalResourceId::new(
        "ferrobridge-synthetic-condition-1",
    )?;
    assert!(
        harness
            .store
            .consumed(
                &ferrobridge_server::facade::identity::record::SourceVersion::new(
                    "Condition",
                    source.clone(),
                    None,
                )
            )?
            .is_none(),
        "a refused commit recorded the source as consumed"
    );
    if let Some(internal) = harness.store.internal_of("Condition", &source)? {
        assert!(
            harness.store.binding_of("Condition", &internal)?.is_none(),
            "a refused commit bound the resource to a composition"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_three_absences_differ_on_the_wire() -> Result<(), Box<dyn StdError>> {
    // The carry-over case of #115: a disabled surface, a type outside the
    // loaded programs and an id of a loaded type that nothing maps are three
    // answers.
    let harness = harness().await;
    let disabled = call(
        ferrobridge_server::router(support::state(), &support::settings()),
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
    )
    .await?;
    let outside = call(
        harness.app(),
        Request::get("/fhir/Observation/abcdef").body(Body::empty())?,
    )
    .await?;
    let unmapped = call(
        harness.app(),
        Request::get("/fhir/Condition/abcdef").body(Body::empty())?,
    )
    .await?;
    for (status, _) in [&disabled, &outside, &unmapped] {
        assert_eq!(StatusCode::NOT_FOUND, *status);
    }
    assert_eq!(
        Some("not-supported"),
        first_issue(&outside.1)["code"].as_str()
    );
    assert_eq!(Some("not-found"), first_issue(&unmapped.1)["code"].as_str());
    let codes = [
        disabled.1["issue"][0]["code"].clone(),
        outside.1["issue"][0]["code"].clone(),
        unmapped.1["issue"][0]["code"].clone(),
    ];
    let diagnostics = [
        disabled.1["issue"][0]["diagnostics"].clone(),
        outside.1["issue"][0]["diagnostics"].clone(),
        unmapped.1["issue"][0]["diagnostics"].clone(),
    ];
    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        assert_ne!(codes[left], codes[right], "{codes:?}");
        assert_ne!(diagnostics[left], diagnostics[right], "{diagnostics:?}");
    }
    Ok(())
}

#[tokio::test]
async fn both_json_media_types_are_read_on_every_write() -> Result<(), Box<dyn StdError>> {
    // "The MIME-type for JSON content is `application/fhir+json`" and
    // `application/json` is its alias (<https://hl7.org/fhir/R4/http.html#mime-type>).
    for media in ["application/fhir+json", "application/json"] {
        let harness = harness().await;
        let id = create_one_as(&harness, media).await?;
        mount_read(&harness.cdr, VERSION_ONE, &harness.composition()).await;
        mount_update(
            &harness.cdr,
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_TWO}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(harness.composition().to_string()),
        )
        .await;
        let updated = call(
            harness.app(),
            Request::put(format!("/fhir/Condition/{id}"))
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(condition().to_string()))?,
        )
        .await?;
        assert_eq!(
            StatusCode::OK,
            updated.0,
            "update as {media}: {}",
            updated.1
        );
        let validated = call(
            harness.app(),
            Request::post(VALIDATE)
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(condition().to_string()))?,
        )
        .await?;
        assert_eq!(
            StatusCode::OK,
            validated.0,
            "$validate as {media}: {}",
            validated.1
        );
        mount_echo(&harness.cdr, ECHOED_BESIDE_CREATE, Echo::Sent).await;
        let mut fresh = condition();
        fresh["id"] = serde_json::json!("a-transaction-entry");
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [
                { "fullUrl": "urn:uuid:0000-good", "resource": fresh,
                  "request": { "method": "POST", "url": "Condition" } }
            ]
        });
        let transaction = call(
            harness.app(),
            Request::post("/fhir")
                .header(header::CONTENT_TYPE, media)
                .body(Body::from(bundle.to_string()))?,
        )
        .await?;
        assert_eq!(
            StatusCode::OK,
            transaction.0,
            "transaction as {media}: {}",
            transaction.1
        );
    }
    Ok(())
}

#[tokio::test]
async fn any_other_media_type_is_four_hundred_and_fifteen_on_every_write()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    for media in ["application/fhir+xml", "text/plain", "application/xml"] {
        for request in [
            Request::post("/fhir/Condition"),
            Request::put("/fhir/Condition/abcdef"),
            Request::post(VALIDATE),
            Request::post("/fhir"),
        ] {
            let (status, body) = call(
                harness.app(),
                request
                    .header(header::CONTENT_TYPE, media)
                    .body(Body::from(condition().to_string()))?,
            )
            .await?;
            assert_eq!(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                status,
                "{media}: {body}"
            );
            assert_eq!(Some("not-supported"), first_issue(&body)["code"].as_str());
        }
    }
    assert_nothing_written(&harness).await;
    Ok(())
}

/// Creates the fixture Condition sent as `media` and returns its logical id.
async fn create_one_as(harness: &Harness, media: &str) -> Result<String, Box<dyn StdError>> {
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(
        harness.app(),
        Request::post("/fhir/Condition")
            .header(header::CONTENT_TYPE, media)
            .body(Body::from(condition().to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::CREATED, status, "create as {media}: {body}");
    Ok(body["id"]
        .as_str()
        .ok_or("the created resource carries an id")?
        .to_owned())
}

#[tokio::test]
async fn the_feeder_audit_carries_the_source_version() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let mut resource = condition();
    resource["meta"]["versionId"] = serde_json::json!("7");
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/composition"))
        .await
        .ok_or("the create sent a composition")?;
    let audit = &sent["feeder_audit"]["originating_system_audit"];
    assert_eq!(
        Some("7"),
        audit["version_id"].as_str(),
        "meta.versionId travels as the source version: {audit}"
    );
    assert_eq!(Some("ferrobridge.test"), audit["system_id"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_source_resource_with_no_id_is_audited_as_unknown() -> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let mut resource = condition();
    if let Some(object) = resource.as_object_mut() {
        object.remove("id");
    }
    let (status, body) = call(harness.app(), post_condition(&resource)).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    let sent = harness
        .sent("POST", &format!("/ehr/{EHR_ID}/composition"))
        .await
        .ok_or("the create sent a composition")?;
    let item = &sent["feeder_audit"]["originating_system_item_ids"][0];
    assert_eq!(
        Some("unknown"),
        item["id"].as_str(),
        "an absent id is recorded as unknown, never invented: {item}"
    );
    assert_eq!(Some("Condition"), item["type"].as_str());
    Ok(())
}

/// Returns the facade settings with `policy` in place of the default one.
fn settings_with(policy: Policy) -> Settings {
    Settings {
        ehr_policy: policy,
        ..settings()
    }
}

/// Mounts the EHR lookup that finds no EHR for any subject.
async fn mount_no_ehr(cdr: &MockServer) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/ehr"))
        .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
            "no EHR for the subject",
        ))
        .mount(cdr)
        .await;
}

#[tokio::test]
async fn an_ehr_created_on_first_sight_names_the_subject_in_the_namespace()
-> Result<(), Box<dyn StdError>> {
    // The subject shape is a PARTY_SELF over a PARTY_REF whose GENERIC_ID
    // scheme is the namespace, which is what `ehr_get_by_subject` matches on
    // (`ehr-codegen.openapi.yaml`).
    let harness = harness_with(settings_with(Policy::CreateOnFirstWrite)).await;
    mount_no_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/ehr"))
        .respond_with(ResponseTemplate::new(201).insert_header("ETag", format!("\"{EHR_ID}\"")))
        .mount(&harness.cdr)
        .await;
    mount_create(&harness.cdr, VERSION_ONE, &harness.composition()).await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::CREATED, status, "{body}");
    let request = harness
        .cdr
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|request| request.method.as_str() == "POST" && request.url.path() == "/ehr")
        .ok_or("the facade created an EHR")?;
    let created: serde_json::Value = serde_json::from_slice(&request.body)?;
    assert_eq!(Some("EHR_STATUS"), created["_type"].as_str(), "{created}");
    let subject = &created["subject"];
    assert_eq!(Some("PARTY_SELF"), subject["_type"].as_str(), "{subject}");
    let reference = &subject["external_ref"];
    assert_eq!(Some("PARTY_REF"), reference["_type"].as_str(), "{subject}");
    assert_eq!(
        Some("http://example.org/fhir/sid/ferrobridge-subject"),
        reference["namespace"].as_str()
    );
    assert_eq!(Some("GENERIC_ID"), reference["id"]["_type"].as_str());
    assert_eq!(
        Some("synthetic-subject-0001"),
        reference["id"]["value"].as_str()
    );
    assert_eq!(
        reference["namespace"], reference["id"]["scheme"],
        "the GENERIC_ID scheme is the namespace: {subject}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_subject_creates_no_ehr_under_the_default_policy()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_no_ehr(&harness.cdr).await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert!(
        status.is_client_error(),
        "an unknown subject under Policy::Existing is refused: {status} {body}"
    );
    assert_eq!(
        0,
        harness.received("POST", "/ehr").await,
        "the facade created an EHR the policy does not allow"
    );
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

#[test]
fn a_compiler_warning_reaches_the_log_once_as_the_facade_loads() -> Result<(), Box<dyn StdError>> {
    // The KDS project context lists an extension of a model no mapping of it
    // reaches, which the compiler accepts with `fc-unreached-extension`.
    let directory = tempfile::tempdir()?;
    crate::kds::write_mappings(directory.path())?;
    let set = programs::read_set(directory.path())?;
    let opt = openehr_its::opt14::from_xml(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT)?;
    let index = openehr_mapping_core::index::WebTemplateIndex::build(
        &openehr_mapping_core::template::TemplateSource::Opt14(Box::new(opt)),
    )?;
    let templates = std::collections::BTreeMap::from([(
        String::from(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_TEMPLATE_ID),
        Arc::new(index),
    )]);
    let logs = support::Logs::default();
    let capture = ferrobridge_server::telemetry::subscriber(
        ferrobridge_server::telemetry::Rendering::Json,
        "warn",
        false,
        logs.clone(),
    );
    let compiled =
        tracing::subscriber::with_default(capture, || programs::compile_set(&set, &templates));
    compiled?;
    let text = logs.text();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("fc-unreached-extension"))
        .collect();
    assert_eq!(lines.len(), 1, "one line per warning: {text}");
    let line: serde_json::Value = serde_json::from_str(lines.first().ok_or("one line")?)?;
    assert_eq!(
        Some("ferrobridge_kds_diagnose.context"),
        line["context"].as_str(),
        "{line}"
    );
    Ok(())
}

/// An error body member no openEHR error document defines, which a rendering
/// of the client's outcome would carry onto the wire.
const UNDOCUMENTED_MEMBER: &str = "ferrobridge-stack-trace-0001";

/// Returns a CDR error body with a `message` and a member outside the
/// `{error, message, validationErrors}` shape.
fn leaky_error_body(message: &str) -> String {
    serde_json::json!({ "message": message, "trace": UNDOCUMENTED_MEMBER }).to_string()
}

#[tokio::test]
async fn a_refused_ehr_creation_carries_only_the_documented_error_shape()
-> Result<(), Box<dyn StdError>> {
    let harness = harness_with(settings_with(Policy::CreateOnFirstWrite)).await;
    mount_no_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/ehr"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("Content-Type", "application/json")
                .set_body_string(leaky_error_body("an EHR for the subject exists")),
        )
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    let diagnostics = diagnostics_of(&body).join(" ");
    assert!(
        diagnostics.contains("an EHR for the subject exists"),
        "the CDR's message travels in the diagnostics: {body}"
    );
    assert!(
        !body.to_string().contains(UNDOCUMENTED_MEMBER),
        "a member outside the openEHR error shape stays off the wire: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_composition_carries_only_the_documented_error_shape()
-> Result<(), Box<dyn StdError>> {
    let harness = harness().await;
    mount_ehr(&harness.cdr).await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("Content-Type", "application/json")
                .set_body_string(leaky_error_body("the composition is malformed")),
        )
        .mount(&harness.cdr)
        .await;
    let (status, body) = call(harness.app(), post_condition(&condition())).await?;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status, "{body}");
    assert!(
        diagnostics_of(&body)
            .join(" ")
            .contains("the composition is malformed"),
        "{body}"
    );
    assert!(
        !body.to_string().contains(UNDOCUMENTED_MEMBER),
        "a member outside the openEHR error shape stays off the wire: {body}"
    );
    Ok(())
}
