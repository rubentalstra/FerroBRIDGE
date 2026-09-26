// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lane itself: absent without a mapping set, and its compiler warnings
//! logged once as it loads.

use axum::Router;
use axum::body::Body;
use ferrobridge_server::config::MappingSettings;
use ferrobridge_server::mappings;
use http::{Request, StatusCode, header};
use std::error::Error as StdError;

use crate::support;

use super::FHIR_JSON;
use super::bundle;
use super::call;
use super::server_settings;

/// Returns the application with no mapping set configured.
fn bare() -> Router {
    ferrobridge_server::router(support::state(), &server_settings())
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

#[test]
fn a_compiler_warning_reaches_the_log_once_as_the_lane_loads() -> Result<(), Box<dyn StdError>> {
    // The KDS project context lists an extension of a model no mapping of it
    // reaches, which the compiler accepts with `fc-unreached-extension`.
    let root = tempfile::tempdir()?;
    crate::kds::write_mappings(&root.path().join("mappings"))?;
    std::fs::create_dir_all(root.path().join("templates"))?;
    std::fs::write(
        root.path().join("templates").join("KDS_Diagnose.opt"),
        ferrobridge_testkit::fixtures::KDS_DIAGNOSE_OPT,
    )?;
    let settings = MappingSettings {
        directory: root.path().join("mappings"),
        templates: root.path().join("templates"),
    };
    let logs = support::Logs::default();
    let capture = ferrobridge_server::telemetry::subscriber(
        ferrobridge_server::telemetry::Rendering::Json,
        "warn",
        false,
        logs.clone(),
    );
    let loaded = tracing::subscriber::with_default(capture, || mappings::load(&settings));
    loaded?;
    let text = logs.text();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("fc-unreached-extension"))
        .collect();
    assert_eq!(lines.len(), 1, "one line per warning: {text}");
    let line: serde_json::Value = serde_json::from_str(lines.first().ok_or("one line")?)?;
    assert_eq!(Some("WARN"), line["level"].as_str(), "{line}");
    assert_eq!(
        Some("ferrobridge_kds_diagnose.context"),
        line["context"].as_str(),
        "{line}"
    );
    assert!(
        line["file"]
            .as_str()
            .is_some_and(|file| file.ends_with("ferrobridge_kds_diagnose.context.yml")),
        "{line}"
    );
    assert!(
        !text.contains("COMPOSITION.report.v1.Condition"),
        "the record names the file and the code, never what the mapping says: {text}"
    );
    Ok(())
}
