// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Where the operations lane takes its templates from at boot.
//!
//! With no `[mappings] templates`, each template a context names is fetched
//! from the CDR at `GET /definition/template/adl1.4/{template_id}` (openEHR
//! ITS-REST 1.1.0 §Definition, `definition-codegen.openapi.yaml`), and a
//! template the CDR does not serve refuses the start.

use ferrobridge_server::config::Config;
use ferrobridge_server::mappings;
use ferrobridge_server::state::AppState;
use http::StatusCode;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers;

/// The mapping set this suite compiles: one context over the testkit template.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/fixtures");

/// The ADL 1.4 route of the template the fixture context names.
const ADL14_ROUTE: &str = "/definition/template/adl1.4/ferrobridge.diagnose.v1";

/// The ADL 2 route the client falls back to on a `404`.
const ADL2_ROUTE: &str = "/definition/template/adl2/ferrobridge.diagnose.v1";

/// Returns the settings `text` describes.
fn settings(text: &str) -> Result<ferrobridge_server::config::Settings, Box<dyn StdError>> {
    Ok(Config::from_sources(Some(text), &BTreeMap::new())?.resolve()?)
}

/// Returns a configuration with a CDR at `cdr` and the fixture mapping set.
fn cdr_text(cdr: &MockServer) -> String {
    format!(
        "[cdr]\nbase_url = \"{}/\"\ntimeout_ms = 2000\n\n[cdr.retry]\nmax_attempts = 1\n\n[mappings]\ndirectory = \"{FIXTURES}\"\n",
        cdr.uri()
    )
}

/// Mounts the fixture template on the ADL 1.4 route of `cdr`.
async fn serve_template(cdr: &MockServer) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path(ADL14_ROUTE))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(ferrobridge_testkit::fixtures::DIAGNOSE_OPT),
        )
        .mount(cdr)
        .await;
}

/// Returns the mapping-set refusal inside a boot refusal.
fn refusal(error: &ferrobridge_server::config::Error) -> Option<&mappings::Error> {
    let mut cause = StdError::source(error);
    while let Some(current) = cause {
        if let Some(found) = current.downcast_ref::<mappings::Error>() {
            return Some(found);
        }
        // A boxed `#[source]` is its own hop in the chain.
        if let Some(found) = current.downcast_ref::<Box<mappings::Error>>() {
            return Some(found);
        }
        cause = current.source();
    }
    None
}

#[tokio::test]
async fn with_no_template_directory_the_lane_compiles_against_the_cdr_templates()
-> Result<(), Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    serve_template(&cdr).await;

    let state = AppState::build(&settings(&cdr_text(&cdr))?).await?;
    let lane = state.operations().ok_or("the operations lane is served")?;
    assert_eq!(1, lane.programs().len());
    let fetched = cdr
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == ADL14_ROUTE)
        .count();
    assert_eq!(1, fetched, "the named template is fetched once");
    Ok(())
}

#[tokio::test]
async fn a_template_the_cdr_does_not_hold_refuses_the_start_naming_it_and_the_status()
-> Result<(), Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    for route in [ADL14_ROUTE, ADL2_ROUTE] {
        Mock::given(matchers::method("GET"))
            .and(matchers::path(route))
            .respond_with(ferrobridge_testkit::stubs::its_rest::not_found(
                "no such template",
            ))
            .mount(&cdr)
            .await;
    }

    let error = AppState::build(&settings(&cdr_text(&cdr))?)
        .await
        .err()
        .ok_or("a template the CDR does not hold refuses the start")?;
    match refusal(&error) {
        Some(mappings::Error::TemplateNotHeld { template, status }) => {
            assert_eq!("ferrobridge.diagnose.v1", template);
            assert_eq!(StatusCode::NOT_FOUND, *status);
        }
        other => panic!("the refusal names the template and the status: {other:?}"),
    }
    let rendered = refusal(&error).map(ToString::to_string).unwrap_or_default();
    assert!(
        rendered.contains("ferrobridge.diagnose.v1") && rendered.contains("404"),
        "{rendered}"
    );
    Ok(())
}

#[tokio::test]
async fn a_template_fetch_the_cdr_refuses_carries_the_upstream_status()
-> Result<(), Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(ADL14_ROUTE))
        .respond_with(ResponseTemplate::new(400))
        .mount(&cdr)
        .await;

    let error = AppState::build(&settings(&cdr_text(&cdr))?)
        .await
        .err()
        .ok_or("a refused template fetch refuses the start")?;
    match refusal(&error) {
        Some(mappings::Error::TemplateRefused { template, upstream }) => {
            assert_eq!("ferrobridge.diagnose.v1", template);
            assert_eq!(StatusCode::BAD_REQUEST, upstream.status());
        }
        other => panic!("the refusal carries the upstream answer: {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn a_cdr_that_fails_the_fetch_refuses_the_start_with_the_status_in_its_cause()
-> Result<(), Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(ADL14_ROUTE))
        .respond_with(ResponseTemplate::new(503))
        .mount(&cdr)
        .await;

    let error = AppState::build(&settings(&cdr_text(&cdr))?)
        .await
        .err()
        .ok_or("a failed template fetch refuses the start")?;
    let Some(found @ mappings::Error::TemplateFetch { template, .. }) = refusal(&error) else {
        panic!("the refusal names the failed fetch: {:?}", refusal(&error));
    };
    assert_eq!("ferrobridge.diagnose.v1", template);
    let cause = found.source().map(ToString::to_string).unwrap_or_default();
    assert!(cause.contains("503"), "{cause}");
    Ok(())
}

#[tokio::test]
async fn a_template_directory_wins_over_the_cdr() -> Result<(), Box<dyn StdError>> {
    let cdr = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(ADL14_ROUTE))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&cdr)
        .await;
    let templates = tempfile::tempdir()?;
    std::fs::write(
        templates.path().join("diagnose.opt"),
        ferrobridge_testkit::fixtures::DIAGNOSE_OPT,
    )?;
    let text = format!(
        "{}templates = \"{}\"\n",
        cdr_text(&cdr),
        templates.path().display()
    );

    let state = AppState::build(&settings(&text)?).await?;
    assert_eq!(
        1,
        state
            .operations()
            .ok_or("the operations lane is served")?
            .programs()
            .len()
    );
    cdr.verify().await;
    Ok(())
}

#[tokio::test]
async fn a_mapping_directory_with_neither_templates_nor_a_cdr_is_refused()
-> Result<(), Box<dyn StdError>> {
    // The refusal comes from resolving the configuration, before any client
    // is built or any CDR is called.
    let text = format!("[mappings]\ndirectory = \"{FIXTURES}\"\n");
    let error = Config::from_sources(Some(&text), &BTreeMap::new())?
        .resolve()
        .err()
        .ok_or("the operations lane has no template source")?;
    assert!(
        matches!(
            refusal(&error),
            Some(mappings::Error::NoTemplateSource { .. })
        ),
        "{error}: {:?}",
        refusal(&error)
    );
    let rendered = refusal(&error).map(ToString::to_string).unwrap_or_default();
    assert!(
        rendered.contains("[mappings] templates") && rendered.contains("[cdr]"),
        "the refusal names both sources: {rendered}"
    );
    Ok(())
}

#[tokio::test]
async fn a_disabled_operations_lane_needs_no_template_source() -> Result<(), Box<dyn StdError>> {
    let text = format!("[mappings]\ndirectory = \"{FIXTURES}\"\n\n[operations]\nenabled = false\n");
    let state = AppState::build(&settings(&text)?).await?;
    assert!(state.operations().is_none());
    Ok(())
}
