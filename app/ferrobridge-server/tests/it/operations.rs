// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIRconnect operations on the wire, through the shipped run path.
//!
//! Every case here reads the DRAFT REST API chapter of the FHIRconnect
//! specification (pull request #93, vendored at its pinned commit under
//! `docs/specs/fhirconnect/draft-rest-api/`), so the expectations move with
//! the chapter when it merges.

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

/// The template the synthetic mapping set compiles against.
const TEMPLATE: &str = "ferrobridge.diagnose.v1";

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

/// Returns the application with no mapping set configured.
fn bare() -> Router {
    ferrobridge_server::router(support::state(), &server_settings())
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

#[tokio::test]
async fn toopenehr_answers_a_parameters_with_the_composition() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("Parameters"), answer["resourceType"].as_str());
    assert_eq!(Some("composition"), answer["parameter"][0]["name"].as_str());
    assert!(
        answer["parameter"][0]["valueString"]
            .as_str()
            .is_some_and(|text| text.contains("Synthetic problem one")),
        "the composition carries the mapped problem name"
    );
    Ok(())
}

#[tokio::test]
async fn tofhir_answers_a_bundle_with_a_provenance() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": built.to_string() }],
    });
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("Bundle"), answer["resourceType"].as_str());
    assert_eq!(Some("collection"), answer["type"].as_str());
    let kinds: Vec<&str> = answer["entry"]
        .as_array()
        .ok_or("the Bundle carries entries")?
        .iter()
        .filter_map(|entry| entry["resource"]["resourceType"].as_str())
        .collect();
    assert!(
        kinds.contains(&"Condition") && kinds.contains(&"Provenance"),
        "the Bundle carries the mapped resource and the Provenance: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "Provenance").count(),
        1,
        "every run carries exactly one Provenance"
    );
    Ok(())
}

#[tokio::test]
async fn the_direct_form_reads_an_openehr_json_body() -> Result<(), Box<dyn StdError>> {
    // "POST [base]/tofhir, Content-Type: application/openehr+json" is the
    // chapter's direct payload invocation, which it keeps outside the FHIR
    // implementation guide.
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/tofhir")
            .header(header::CONTENT_TYPE, OPENEHR_JSON)
            .body(Body::from(built.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("Bundle"), answer["resourceType"].as_str());
    Ok(())
}

#[tokio::test]
async fn the_direct_form_answers_the_composition_itself() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(OPENEHR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("COMPOSITION"), answer["_type"].as_str());
    Ok(())
}

#[tokio::test]
async fn the_direct_form_answers_the_requested_format() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/toopenehr?format=flat")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    assert_eq!(OPENEHR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert!(
        answer["_type"].is_null(),
        "a flat document carries no _type: {body}"
    );
    assert!(
        answer
            .as_object()
            .is_some_and(|members| members.keys().any(|key| key.contains('/'))),
        "a flat document keys its values by template path"
    );
    Ok(())
}

#[tokio::test]
async fn the_body_takes_precedence_over_the_query() -> Result<(), Box<dyn StdError>> {
    // "Where the same field is supplied both in the body and as a query
    // parameter, the body takes precedence" (rest-api.adoc, draft, section
    // Query parameters).
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            { "name": "composition", "valueString": built.to_string() },
            { "name": "templateId", "valueString": TEMPLATE },
        ],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir?templateId=not.loaded.v1")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(
        StatusCode::OK,
        status,
        "the body's templateId wins over the query's: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn the_query_pins_the_template_when_the_body_does_not() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let built = composition(app(&root)?).await?;
    let flat = call(
        app(&root)?,
        Request::post("/fhir/toopenehr?format=flat")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, flat.0, "{}", flat.2);
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": flat.2 }],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir?templateId=not.loaded.v1")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(
        StatusCode::BAD_REQUEST,
        status,
        "the query's templateId reached the run: {body}"
    );
    assert!(
        body.contains("not.loaded.v1"),
        "the outcome names the template the query pinned: {body}"
    );
    drop(built);
    Ok(())
}

#[tokio::test]
async fn a_flat_composition_without_a_template_is_four_hundred_required()
-> Result<(), Box<dyn StdError>> {
    // "`templateId` ... is also required when submitting a flat Composition to
    // `$tofhir`" (rest-api.adoc, draft, section Query parameters).
    let root = mapping_tree()?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "composition",
            "valueString": r#"{"diagnose/problem_diagnosis/problem_diagnosis|value":"a"}"#,
        }],
    });
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status, "{body}");
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    assert_eq!(Some("required"), answer["issue"][0]["code"].as_str());
    Ok(())
}

#[tokio::test]
async fn another_media_type_is_four_hundred_fifteen() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    for route in [
        "/fhir/$tofhir",
        "/fhir/$toopenehr",
        "/fhir/tofhir",
        "/fhir/toopenehr",
    ] {
        let (status, media, body) = call(
            app(&root)?,
            Request::post(route)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("not a FHIR body"))?,
        )
        .await?;
        assert_eq!(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            status,
            "{route} read a text/plain body"
        );
        assert_eq!(FHIR_JSON, media, "{route}");
        let answer: serde_json::Value = serde_json::from_str(&body)?;
        assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    }
    Ok(())
}

#[tokio::test]
async fn an_openehr_body_on_the_enveloped_route_is_four_hundred_fifteen()
-> Result<(), Box<dyn StdError>> {
    // "The `Content-Type` and `Accept` headers on the operation endpoints ...
    // refer to the FHIR envelope, not to the openEHR payload it carries"
    // (rest-api.adoc, draft, section Media types).
    let root = mapping_tree()?;
    let (status, _, _) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, OPENEHR_JSON)
            .body(Body::from("{}"))?,
    )
    .await?;
    assert_eq!(StatusCode::UNSUPPORTED_MEDIA_TYPE, status);
    Ok(())
}

#[tokio::test]
async fn a_body_that_is_not_a_parameters_answers_an_operation_outcome()
-> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let (status, media, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from("not json at all"))?,
    )
    .await?;
    assert_eq!(StatusCode::BAD_REQUEST, status);
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    assert_eq!(Some("error"), answer["issue"][0]["severity"].as_str());
    Ok(())
}

#[tokio::test]
async fn a_mapping_that_cannot_run_answers_an_outcome_and_nothing_else()
-> Result<(), Box<dyn StdError>> {
    // Strictness is the default: a failed mapping answers an OperationOutcome
    // and no Bundle.
    let root = mapping_tree()?;
    let mut built = composition(app(&root)?).await?;
    if let Some(object) = built.as_object_mut() {
        object.remove("uid");
    }
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": built.to_string() }],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    assert!(
        answer["entry"].is_null() && answer["parameter"].is_null(),
        "a refusal carries no Bundle and no composition: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_bundle_with_two_subjects_is_refused_naming_them() -> Result<(), Box<dyn StdError>> {
    let root = mapping_tree()?;
    let mut first: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::R4_CONDITION)?;
    let mut second = first.clone();
    for (resource, subject, id) in [
        (&mut first, "Patient/one", "one"),
        (&mut second, "Patient/two", "two"),
    ] {
        if let Some(object) = resource.as_object_mut() {
            object.insert(String::from("id"), serde_json::json!(id));
            object.insert(
                String::from("meta"),
                serde_json::json!({ "profile": [PROFILE] }),
            );
            object.insert(
                String::from("subject"),
                serde_json::json!({ "reference": subject }),
            );
        }
    }
    let carried = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{ "resource": first }, { "resource": second }],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(carried.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, status, "{body}");
    assert!(
        body.contains("Patient/one") && body.contains("Patient/two"),
        "the outcome names the conflicting subjects: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn the_operations_answer_unavailable_with_no_mapping_set() -> Result<(), Box<dyn StdError>> {
    let (status, media, body) = call(
        bare(),
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(bundle()?))?,
    )
    .await?;
    assert_eq!(StatusCode::SERVICE_UNAVAILABLE, status);
    assert_eq!(FHIR_JSON, media);
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(Some("OperationOutcome"), answer["resourceType"].as_str());
    Ok(())
}

#[tokio::test]
async fn the_enveloped_toopenehr_reads_a_parameters_body() -> Result<(), Box<dyn StdError>> {
    // ToOpenEhr.fsh declares `bundle` as an `in` parameter while the chapter's
    // prose puts the Bundle in the body directly; both forms are read.
    let root = mapping_tree()?;
    let carried: serde_json::Value = serde_json::from_str(&bundle()?)?;
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            { "name": "bundle", "resource": carried },
            { "name": "format", "valueCode": "flat" },
        ],
    });
    let (status, _, body) = call(
        app(&root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    let text = composition_text(&answer)?;
    assert!(
        text.contains('/'),
        "the format parameter selected the flat serialization: {text}"
    );
    Ok(())
}

/// Returns the one `Condition` a `$tofhir` Bundle carries.
fn mapped_condition(answer: &serde_json::Value) -> Result<&serde_json::Value, Box<dyn StdError>> {
    answer["entry"]
        .as_array()
        .ok_or("the Bundle carries entries")?
        .iter()
        .map(|entry| &entry["resource"])
        .find(|resource| resource["resourceType"].as_str() == Some("Condition"))
        .ok_or_else(|| Box::<dyn StdError>::from("the Bundle carries the mapped Condition"))
}

/// Runs `$toopenehr` then `$tofhir` over `condition` and returns what the
/// second leg mapped back.
async fn both_legs(
    root: &tempfile::TempDir,
    condition: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn StdError>> {
    let carried = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{ "resource": condition }],
    });
    let (status, _, body) = call(
        app(root)?,
        Request::post("/fhir/$toopenehr")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(carried.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    let mut built: serde_json::Value = serde_json::from_str(composition_text(&answer)?)?;
    // NOTE: no specification governs this: our own design, a composition
    // takes its `uid` when a CDR commits it, so the round trip without one
    // gives it the version a commit would, as `composition` above does.
    if let Some(object) = built.as_object_mut() {
        object.insert(
            String::from("uid"),
            serde_json::json!({
                "_type": "OBJECT_VERSION_ID",
                "value": "d2b3c1a0-0000-4000-8000-000000000001::ferrobridge.example::1",
            }),
        );
    }
    let request = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [{ "name": "composition", "valueString": built.to_string() }],
    });
    let (status, _, body) = call(
        app(root)?,
        Request::post("/fhir/$tofhir")
            .header(header::CONTENT_TYPE, FHIR_JSON)
            .body(Body::from(request.to_string()))?,
    )
    .await?;
    assert_eq!(StatusCode::OK, status, "{body}");
    let answer: serde_json::Value = serde_json::from_str(&body)?;
    Ok(mapped_condition(&answer)?.clone())
}

#[tokio::test]
async fn the_two_operations_round_trip_a_condition_modulo_what_the_program_maps()
-> Result<(), Box<dyn StdError>> {
    // The synthetic program maps the diagnosis name both ways and writes the
    // subject going to FHIR only, so every other element of the input is the
    // declared loss of this round trip, the subject comes back as the literal
    // reference the program writes, and the id is the one `$tofhir` derives
    // from the composition's version.
    let root = mapping_tree()?;
    let input =
        serde_json::from_str::<serde_json::Value>(&bundle()?)?["entry"][0]["resource"].clone();
    let output = both_legs(&root, &input).await?;
    let set = ferrobridge_testkit::laws::declared(&[], &[], &input, &output);
    assert_eq!(
        set["lost"],
        serde_json::json!([
            "clinicalStatus.coding[0].code = \"active\"",
            "clinicalStatus.coding[0].display = \"Active\"",
            "clinicalStatus.coding[0].system = \"http://terminology.hl7.org/CodeSystem/condition-clinical\"",
            "identifier[0].system = \"http://example.org/fhir/sid/ferrobridge-condition\"",
            "identifier[0].value = \"synthetic-condition-0001\"",
            format!("meta.profile[0] = \"{PROFILE}\""),
            "recordedDate = \"2026-09-12T10:00:00+02:00\"",
            "subject.identifier.system = \"http://example.org/fhir/sid/ferrobridge-subject\"",
            "subject.identifier.value = \"synthetic-subject-0001\"",
            "verificationStatus.coding[0].code = \"confirmed\"",
            "verificationStatus.coding[0].display = \"Confirmed\"",
            "verificationStatus.coding[0].system = \"http://terminology.hl7.org/CodeSystem/condition-ver-status\"",
        ]),
        "{set}"
    );
    assert_eq!(
        set["added"],
        serde_json::json!(["subject.reference = \"Patient/synthetic-subject-0001\""]),
        "{set}"
    );
    assert_eq!(
        set["changed"],
        serde_json::json!([
            "id: \"ferrobridge-synthetic-condition-1\" became \"d2b3c1a0-0000-4000-8000-000000000001\""
        ]),
        "{set}"
    );
    Ok(())
}

// TODO(#86): run once the engine closes four gaps. G1: an untyped mapping onto
// a primitive dateTime takes the string kind, so no DV_DATE_TIME cell reads it.
// G2: an untyped mapping onto a structural node runs a data cell instead of
// anchoring its children. G3: a choice element with no type filter takes no
// kind from the instance. G4: the required-child check counts a structural
// anchor as a missing value.
#[tokio::test]
#[ignore = "the engine refuses the published chain; see the TODO above"]
async fn the_two_operations_round_trip_the_kds_condition_as_the_engine_does()
-> Result<(), Box<dyn StdError>> {
    let root = tempfile::tempdir()?;
    crate::kds::write_mappings(&root.path().join("mappings"))?;
    std::fs::create_dir_all(root.path().join("templates"))?;
    std::fs::write(
        root.path().join("templates").join("KDS_Diagnose.opt"),
        ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT,
    )?;
    let input: serde_json::Value =
        serde_json::from_str(ferrobridge_testkit::fixtures::KDS_DIAGNOSE_CONDITION)?;
    let output = both_legs(&root, &input).await?;
    let set = ferrobridge_testkit::laws::declared(&[], &[], &input, &output);
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/fhirconnect/tests/it/snapshots/it__roundtrip__kds_putget.snap"
    ))?;
    let body = pinned
        .splitn(3, "---")
        .nth(2)
        .ok_or("the snapshot carries a body")?;
    let engine: serde_json::Value = serde_json::from_str(body)?;
    for list in ["lost", "added", "changed"] {
        assert_eq!(
            set[list], engine[list],
            "the wire and the engine disagree on {list}"
        );
    }
    Ok(())
}
