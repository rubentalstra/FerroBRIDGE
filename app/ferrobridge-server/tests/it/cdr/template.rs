// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two-route template fetch over the generated client: ADL 1.4 first,
//! ADL 2 on a `404` or a `406`.

use super::support;
use ferrobridge_server::cdr::error::CdrError;
use ferrobridge_server::cdr::ids::template_id;
use ferrobridge_server::cdr::template::{TemplateOutcome, TemplateSource};
use std::error::Error;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The template identifier every case in this module asks for.
const TEMPLATE: &str = "Synthetic vital signs";
/// The percent-encoded path of that identifier on the `adl1.4` route.
const ADL14_PATH: &str = "/v1/definition/template/adl1.4/Synthetic%20vital%20signs";
/// The percent-encoded path of that identifier on the `adl2` route.
const ADL2_PATH: &str = "/v1/definition/template/adl2/Synthetic%20vital%20signs";

#[tokio::test]
async fn the_adl1_4_route_answers_the_canonical_opt() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ADL14_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(support::OPT14_XML),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .template(&template_id(TEMPLATE)?)
        .await?;
    match outcome {
        TemplateOutcome::Found(TemplateSource::Opt14(template)) => {
            assert_eq!(TEMPLATE, template.template_id.value);
        }
        other => return Err(format!("expected an OPT 1.4 template, got {other:?}").into()),
    }
    assert_eq!(
        vec!["application/xml".to_owned()],
        support::request_header(&server, 0, "accept").await?
    );
    assert_eq!(
        vec!["return=representation".to_owned()],
        support::request_header(&server, 0, "prefer").await?
    );
    Ok(())
}

#[tokio::test]
async fn a_404_on_adl1_4_falls_back_to_adl2() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ADL14_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADL2_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{}\"", support::OPT2_HRID))
                .set_body_string(support::OPT2_JSON),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .template(&template_id(TEMPLATE)?)
        .await?;
    match outcome {
        TemplateOutcome::Found(TemplateSource::Opt2 { resolved_id, .. }) => {
            assert_eq!(support::OPT2_HRID, resolved_id.physical_id());
        }
        other => return Err(format!("expected an ADL 2 template, got {other:?}").into()),
    }
    assert_eq!(
        vec!["application/json".to_owned()],
        support::request_header(&server, 1, "accept").await?
    );
    Ok(())
}

#[tokio::test]
async fn a_406_on_adl1_4_falls_back_to_adl2() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ADL14_PATH))
        .respond_with(ResponseTemplate::new(406))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADL2_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{}\"", support::OPT2_HRID))
                .set_body_string(support::OPT2_JSON),
        )
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .template(&template_id(TEMPLATE)?)
        .await?;
    assert!(matches!(
        outcome,
        TemplateOutcome::Found(TemplateSource::Opt2 { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn the_adl2_request_never_offers_text_plain() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ADL14_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADL2_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", format!("W/\"{}\"", support::OPT2_HRID))
                .set_body_string(support::OPT2_JSON),
        )
        .mount(&server)
        .await;

    let _outcome = support::client(&server)?
        .template(&template_id(TEMPLATE)?)
        .await?;
    let accept = support::request_header(&server, 1, "accept").await?;
    assert_eq!(vec!["application/json".to_owned()], accept);
    assert!(!accept.iter().any(|value| value.contains("text/plain")));
    assert!(!accept.iter().any(|value| value.contains("application/xml")));
    Ok(())
}

#[tokio::test]
async fn an_adl2_body_of_the_wrong_type_is_refused() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ADL14_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADL2_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"_type":"AUTHORED_ARCHETYPE","archetype_id":{}}"#),
        )
        .mount(&server)
        .await;

    let error = support::client(&server)?
        .template(&template_id(TEMPLATE)?)
        .await
        .expect_err("a body that is not an operational template is refused");
    match error {
        CdrError::NotOperationalTemplate { found } => {
            assert_eq!(Some("AUTHORED_ARCHETYPE".to_owned()), found);
        }
        other => return Err(format!("expected a refusal, got {other:?}").into()),
    }
    Ok(())
}

#[tokio::test]
async fn a_miss_on_both_routes_is_the_unknown_template_outcome() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ADL14_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ADL2_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let outcome = support::client(&server)?
        .template(&template_id(TEMPLATE)?)
        .await?;
    assert!(matches!(outcome, TemplateOutcome::UnknownTemplate));
    Ok(())
}
