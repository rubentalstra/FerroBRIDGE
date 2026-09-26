// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect operations on the wire, through the shipped run path.
//!
//! Every case here reads the DRAFT REST API chapter of the FHIRconnect
//! specification (pull request #93, vendored at its pinned commit under
//! `docs/specs/fhirconnect/draft-rest-api/`), so the expectations move with
//! the chapter when it merges.

mod answers;
mod lane;
mod refusals;
mod round_trip;

use axum::Router;
use axum::body::Body;
use ferrobridge_server::config::MappingSettings;
use ferrobridge_server::health::Registry;
use ferrobridge_server::mappings;
use ferrobridge_server::state::AppState;
use ferrobridge_server::state::OperationsLane;
use http::{Request, Response, StatusCode, header};
use std::error::Error as StdError;
use std::sync::Arc;
use tower::ServiceExt as _;

use crate::support;

/// The FHIR JSON media type both operations exchange.
const FHIR_JSON: &str = "application/fhir+json";

/// The media type the draft defines for an unwrapped openEHR payload.
const OPENEHR_JSON: &str = "application/openehr+json";

/// The profile the synthetic context claims.
const PROFILE: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-server-diagnose";

/// A synthetic context mapping over the testkit's diagnosis template.
const CONTEXT_FILE: &str = r#"grammar: FHIRConnect/v1.0.0
type: context
metadata:
  name: ferrobridge_server_diagnose.context
  version: 1.0.0
spec:
  system: FHIR
  version: R4
context:
  profile:
    url: "http://example.org/fhir/StructureDefinition/ferrobridge-server-diagnose"
  template:
    id: "ferrobridge.diagnose.v1"
  archetypes:
    - "ferrobridge_server_diagnose"
  start: "ferrobridge_server_diagnose"
"#;

/// A synthetic model mapping: the problem name and the subject.
const MODEL_FILE: &str = r#"grammar: FHIRConnect/v1.0.0
type: model
metadata:
  name: ferrobridge_server_diagnose
  version: 1.0.0
spec:
  system: FHIR
  version: R4
  openEhrConfig:
    archetype: openEHR-EHR-EVALUATION.problem_diagnosis.v1
  fhirConfig:
    structureDefinition: http://hl7.org/fhir/StructureDefinition/Condition

mappings:
  - name: "problemDiagnose"
    with:
      fhir: "$resource.code"
      openehr: "$archetype/data[at0001]/items[at0002]"
      type: "CODEABLECONCEPT"

  - name: "subject"
    unidirectional: "openehr->fhir"
    with:
      fhir: "$resource.subject"
    manual:
      - name: "subjectReference"
        fhir:
          - path: "reference"
            value: "Patient/synthetic-subject-0001"
"#;

/// Writes the synthetic mapping set into a temporary tree.
fn mapping_tree() -> Result<tempfile::TempDir, Box<dyn StdError>> {
    let root = tempfile::tempdir()?;
    let mappings = root.path().join("mappings");
    let templates = root.path().join("templates");
    std::fs::create_dir_all(&mappings)?;
    std::fs::create_dir_all(&templates)?;
    std::fs::write(
        mappings.join("ferrobridge_server_diagnose.context.yml"),
        CONTEXT_FILE,
    )?;
    std::fs::write(mappings.join("ferrobridge_server_diagnose.yml"), MODEL_FILE)?;
    std::fs::write(
        templates.join("diagnose.opt"),
        ferrobridge_testkit::fixtures::DIAGNOSE_OPT,
    )?;
    Ok(root)
}

/// Returns the application with the synthetic mapping set loaded.
fn app(root: &tempfile::TempDir) -> Result<Router, Box<dyn StdError>> {
    let settings = MappingSettings {
        directory: root.path().join("mappings"),
        templates: root.path().join("templates"),
    };
    let programs = mappings::load(&settings)?;
    let lane = OperationsLane::new(programs, "Device/ferrobridge-test").with_composition_defaults(
        "FHIRconnect",
        Some(String::from("en")),
        Some(String::from("NL")),
    );
    let state = Arc::new(AppState::with_health(Registry::default()).serving_operations(lane));
    Ok(ferrobridge_server::router(state, &server_settings()))
}

/// Returns server settings whose body ceiling admits a composition.
///
/// A clinical document is larger than the shared test ceiling, so the
/// operations run under the default one the server ships.
fn server_settings() -> ferrobridge_server::config::ServerSettings {
    ferrobridge_server::config::ServerSettings {
        body_limit: 1024 * 1024,
        ..support::settings()
    }
}

/// Sends `request` through `app` and reads the status, the content type and
/// the body.
async fn call(
    app: Router,
    request: Request<Body>,
) -> Result<(StatusCode, String, String), Box<dyn StdError>> {
    let response: Response<Body> = app.oneshot(request).await?;
    let status = response.status();
    let media = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, media, String::from_utf8(bytes.to_vec())?))
}

/// Returns the synthetic `Condition` of the testkit inside a `collection`
/// Bundle claiming the synthetic profile.
fn bundle() -> Result<String, Box<dyn StdError>> {
    let mut condition: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    if let Some(object) = condition.as_object_mut() {
        object.insert(
            String::from("meta"),
            serde_json::json!({ "profile": [PROFILE] }),
        );
    }
    Ok(serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{ "resource": condition }],
    })
    .to_string())
}

/// Returns the composition string the first answered parameter carries.
fn composition_text(answer: &serde_json::Value) -> Result<&str, Box<dyn StdError>> {
    answer
        .get("parameter")
        .and_then(|parameter| parameter.get(0))
        .and_then(|first| first.get("valueString"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Box::<dyn StdError>::from("the answer carries the composition"))
}

/// Maps the synthetic Bundle into a composition through `$toopenehr`.
async fn composition(app: Router) -> Result<serde_json::Value, Box<dyn StdError>> {
    let (status, _, body) = call(
        app,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    let text = composition_text(&answer)?;
    let mut built: serde_json::Value = serde_json::from_str(text)?;
    if let Some(object) = built.as_object_mut() {
        object.insert(
            String::from("uid"),
            serde_json::json!({
                "_type": "OBJECT_VERSION_ID",
                "value": "d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1",
            }),
        );
    }
    Ok(built)
}
