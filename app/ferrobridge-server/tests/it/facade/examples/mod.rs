// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR R4 examples package through the facade, one case per context and
//! example of a resource type the context maps.
//!
//! Each example is created with the context's profile claimed and read back
//! from a `wiremock` CDR that keeps the composition it was sent. A case
//! passes when the read is an instance the R4 model admits, no element the
//! example carries comes back with another value (`PutGet`, over the leaves
//! [`ferrobridge_testkit::laws::declared`] compares), and a second create and
//! read of that answer gives it back unchanged. What a mapping does not reach
//! is lost, and is counted beside the verdict. An example of a type no
//! context maps is set aside by type, never failed. The identity, the version
//! and the subject are the facade's own and leave every comparison. No
//! specification governs the instrument: our own design.

mod e2e;

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use axum::body::Body;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::Settings;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::programs;
use ferrobridge_server::facade::programs::Loaded;
use ferrobridge_server::facade::programs::Programs;
use ferrobridge_server::health::Registry;
use ferrobridge_server::state::AppState;
use ferrobridge_testkit::conformance::Case;
use ferrobridge_testkit::conformance::Corpus;
use ferrobridge_testkit::conformance::record_set_aside;
use ferrobridge_testkit::examples::Package;
use fhir_types::codec::Json;
use http::Request;
use http::StatusCode;
use http::header;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

use super::EHR_ID;
use super::VERSION_ONE;
use super::call;
use super::client;
use super::diagnostics_of;
use super::handle;
use super::server_settings;
use super::stub::mount_ehr;

/// A context the examples are run through.
#[derive(Debug, Clone, Copy)]
enum Context {
    /// The suite's own context over the synthetic diagnosis template.
    Suite,
    /// The KDS diagnosis context over the published `KDS_Diagnose` template.
    Kds,
}

impl Context {
    /// Every context, in the order the cases are reported.
    const ALL: [Self; 2] = [Self::Suite, Self::Kds];

    /// Returns the name that opens the context's case ids.
    const fn id(self) -> &'static str {
        match self {
            Self::Suite => "ferrobridge_facade",
            Self::Kds => "kds_diagnose",
        }
    }

    /// Returns the profile the context claims.
    const fn profile(self) -> &'static str {
        match self {
            Self::Suite => super::PROFILE,
            Self::Kds => crate::kds::PROFILE,
        }
    }

    /// Returns the facade settings the context runs with.
    fn settings(self) -> Settings {
        match self {
            Self::Suite => super::settings(),
            Self::Kds => Settings {
                language: String::from("de"),
                territory: String::from("DE"),
                ..super::settings()
            },
        }
    }
}

/// One context compiled once, with the settings every case of it runs with.
struct Compiled {
    /// The context.
    context: Context,
    /// The compiled program and its template index.
    loaded: Vec<Loaded>,
}

impl Compiled {
    /// Returns the resource types the context maps.
    fn serves(&self, resource_type: &str) -> bool {
        self.loaded
            .iter()
            .any(|entry| entry.program().resource().as_str() == resource_type)
    }
}

/// Compiles `context` against a stub CDR that serves its template.
async fn compile(context: Context) -> Result<Compiled, Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    for (id, opt) in [
        (
            "ferrobridge.diagnose.v1",
            ferrobridge_testkit::fixtures::DIAGNOSE_OPT,
        ),
        (
            "KDS_Diagnose",
            ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT,
        ),
    ] {
        Mock::given(matchers::method("GET"))
            .and(matchers::path(format!("/definition/template/adl1.4/{id}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/xml")
                    .set_body_string(opt),
            )
            .mount(&cdr)
            .await;
    }
    let directory = tempfile::tempdir()?;
    let mappings = match context {
        Context::Suite => Path::new(super::FIXTURES).to_path_buf(),
        Context::Kds => {
            crate::kds::write_mappings(directory.path())?;
            directory.path().to_path_buf()
        }
    };
    let set = programs::read_set(&mappings)?;
    let templates = programs::fetch_templates(&set, &client(&cdr)).await?;
    let compiled = programs::compile_set(&set, &templates)?;
    Ok(Compiled {
        context,
        loaded: compiled.loaded().to_vec(),
    })
}

/// The stub CDR half that keeps the composition a create sends and echoes it.
struct Keep {
    /// The composition last sent.
    kept: Arc<Mutex<Option<String>>>,
}

impl wiremock::Respond for Keep {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body).into_owned();
        let Ok(mut kept) = self.kept.lock() else {
            return ResponseTemplate::new(500);
        };
        *kept = Some(body.clone());
        ResponseTemplate::new(201)
            .insert_header("ETag", format!("W/\"{VERSION_ONE}\""))
            .insert_header("Content-Type", "application/json")
            .set_body_string(body)
    }
}

/// The stub CDR half that answers every composition read with the kept one.
struct Serve {
    /// The composition the create kept.
    kept: Arc<Mutex<Option<String>>>,
}

impl wiremock::Respond for Serve {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let Ok(kept) = self.kept.lock() else {
            return ResponseTemplate::new(500);
        };
        match kept.as_ref() {
            Some(body) => ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{VERSION_ONE}\""))
                .insert_header("Content-Type", "application/json")
                .set_body_string(body.clone()),
            None => ResponseTemplate::new(404),
        }
    }
}

/// Returns `resource` with `profile` as the one profile it claims.
fn claiming(resource: &serde_json::Value, profile: &str) -> serde_json::Value {
    let mut claimed = resource.clone();
    claimed["meta"]["profile"] = serde_json::json!([profile]);
    claimed
}

/// Creates `resource` through a fresh facade over a fresh stub CDR and reads
/// it back, or answers why the trip stopped.
async fn trip(
    compiled: &Compiled,
    resource: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let cdr = MockServer::start().await;
    mount_ehr(&cdr).await;
    let kept = Arc::new(Mutex::new(None));
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("/ehr/{EHR_ID}/composition")))
        .respond_with(Keep {
            kept: Arc::clone(&kept),
        })
        .mount(&cdr)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path_regex(format!(
            "^/ehr/{EHR_ID}/composition/.+$"
        )))
        .respond_with(Serve { kept })
        .mount(&cdr)
        .await;
    let facade = Arc::new(Facade::new(
        Programs::new(compiled.loaded.clone()),
        handle(&Arc::new(MemoryStore::new())),
        client(&cdr),
        compiled.context.settings(),
    ));
    let state = AppState::with_health(Registry::default()).with_facade(facade);
    let app = ferrobridge_server::router(Arc::new(state), &server_settings());
    let resource_type = resource["resourceType"].as_str().unwrap_or_default();
    let claimed = claiming(resource, compiled.context.profile());
    let create = Request::post(format!("/fhir/{resource_type}"))
        .header(header::CONTENT_TYPE, "application/fhir+json")
        .body(Body::from(claimed.to_string()))
        .map_err(|error| format!("the create request does not build: {error}"))?;
    let (status, created) = call(app.clone(), create)
        .await
        .map_err(|error| format!("the create did not answer: {error}"))?;
    if status != StatusCode::CREATED {
        return Err(format!(
            "the create answered {status}: {}",
            diagnostics_of(&created).join("; ")
        ));
    }
    let id = created["id"]
        .as_str()
        .ok_or("the created resource carries no id")?;
    let read = Request::get(format!("/fhir/{resource_type}/{id}"))
        .body(Body::empty())
        .map_err(|error| format!("the read request does not build: {error}"))?;
    let (status, body) = call(app, read)
        .await
        .map_err(|error| format!("the read did not answer: {error}"))?;
    if status != StatusCode::OK {
        return Err(format!(
            "the read answered {status}: {}",
            diagnostics_of(&body).join("; ")
        ));
    }
    Ok(body)
}

/// Returns `document` without what the facade writes of its own.
fn compared(document: &serde_json::Value) -> serde_json::Value {
    // NOTE: no specification governs this: our own design, the identity, the
    // version and the subject are the facade's, so they leave the comparison.
    let mut stripped = document.clone();
    if let Some(object) = stripped.as_object_mut() {
        object.remove("id");
        object.remove("meta");
        object.remove("subject");
    }
    stripped
}

/// Returns why `read` is no instance the R4 model admits, if it is not.
fn invalid(read: &serde_json::Value) -> Option<String> {
    let document = fhir_types::codec::Value::from_serde_json(read.clone());
    let mut path = fhir_types::codec::Path::root("Resource");
    fhir_types::codec::expect_object(&document, &path)
        .and_then(|object| fhir_types::r4::resource::Resource::from_json(object, &mut path))
        .err()
        .map(|error| format!("the read is no R4 instance: {error}"))
}

/// Returns the first entry of `list` in a declared set, if it has one.
fn first(set: &serde_json::Value, list: &str) -> Option<String> {
    set[list]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Runs one example through one context and returns its verdict.
async fn case(compiled: &Compiled, id: String, resource: &serde_json::Value) -> Case {
    let read = match trip(compiled, resource).await {
        Ok(read) => read,
        Err(reason) => return Case::fail(id, reason),
    };
    if let Some(reason) = invalid(&read) {
        return Case::fail(id, reason);
    }
    let putget =
        ferrobridge_testkit::laws::declared(&[], &[], &compared(resource), &compared(&read));
    let outcomes: BTreeMap<String, usize> = ["lost", "added"]
        .into_iter()
        .map(|list| {
            let count = putget[list].as_array().map_or(0, Vec::len);
            (String::from(list), count)
        })
        .collect();
    if let Some(changed) = first(&putget, "changed") {
        return Case::fail(id, format!("PutGet changes {changed}")).with_outcomes(outcomes);
    }
    let again = match trip(compiled, &read).await {
        Ok(again) => again,
        Err(reason) => {
            return Case::fail(id, format!("the second round trip: {reason}"))
                .with_outcomes(outcomes);
        }
    };
    let moved = ferrobridge_testkit::laws::declared(&[], &[], &compared(&read), &compared(&again));
    let difference = ["lost", "added", "changed"]
        .into_iter()
        .find_map(|list| first(&moved, list).map(|row| format!("{list} {row}")));
    match difference {
        Some(row) => {
            Case::fail(id, format!("the second round trip moved: {row}")).with_outcomes(outcomes)
        }
        None => Case::pass(id).with_outcomes(outcomes),
    }
}

/// Whether the R4 examples are on disk, or why the corpus skips.
#[expect(
    clippy::print_stderr,
    reason = "a corpus whose build-time package is absent says it skipped"
)]
fn fetched() -> bool {
    let present = Package::R4.fetched();
    if !present {
        eprintln!(
            "skipped: {} is fetched at build time; run scripts/vendor/fhir-packages.sh --build-time",
            Package::R4
        );
    }
    present
}

#[tokio::test(flavor = "multi_thread")]
async fn conformance_the_fhir_r4_examples_hold_their_facade_pass_list()
-> Result<(), Box<dyn StdError>> {
    if !fetched() {
        return Ok(());
    }
    let mut contexts = Vec::new();
    for context in Context::ALL {
        contexts.push(compile(context).await?);
    }
    let mut cases = Vec::new();
    let mut set_aside: BTreeMap<String, usize> = BTreeMap::new();
    for file in Package::R4.resources()? {
        let name = file
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or("an example file has a UTF-8 name")?
            .to_owned();
        let resource: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        let resource_type = resource["resourceType"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let serving: Vec<&Compiled> = contexts
            .iter()
            .filter(|c| c.serves(&resource_type))
            .collect();
        if serving.is_empty() {
            *set_aside.entry(resource_type).or_default() += 1;
            continue;
        }
        for compiled in serving {
            cases.push(
                case(
                    compiled,
                    format!("{}/{name}", compiled.context.id()),
                    &resource,
                )
                .await,
            );
        }
    }
    assert!(
        !cases.is_empty(),
        "no R4 example is of a type a context maps"
    );
    let outcome = record_set_aside(Corpus::FhirR4Facade, &cases, &set_aside)?;
    assert!(
        outcome.regressed.is_empty(),
        "cases the {} pass list records no longer pass: {:?}",
        Corpus::FhirR4Facade,
        outcome.regressed
    );
    Ok(())
}
