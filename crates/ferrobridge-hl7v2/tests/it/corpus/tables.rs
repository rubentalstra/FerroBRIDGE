// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A terminology stub answering `$translate` from the guide's own table maps.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Duration;

use ferrobridge_term::client::Client;
use ferrobridge_term::config::{Config, RetryPolicy, WireVersion};
use ferrobridge_testkit::stubs::terminology;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A `$translate` question: the table map's url, the source system, the code
/// and the target system.
type Question = (String, String, String, String);

/// One target of a table map row: its equivalence, code and display.
type Answer = (String, String, String);

/// A terminology server loaded with the guide's own table maps: it answers
/// `$translate` from the `ConceptMap` the request names, by source system,
/// code and target system, as a server holding those maps does.
#[derive(Debug, Clone, Default)]
pub(super) struct GuideTables {
    answers: BTreeMap<Question, Vec<Answer>>,
}

impl GuideTables {
    /// Reads every `ConceptMap-table-*.json` of the package.
    pub(super) fn load(package: &Path) -> Result<Self, Box<dyn Error>> {
        let mut answers: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for entry in fs::read_dir(package)? {
            let path = entry?.path();
            let is_table = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("ConceptMap-table-"));
            if !is_table {
                continue;
            }
            let map: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            let url = map["url"].as_str().unwrap_or_default().to_owned();
            for group in map["group"].as_array().into_iter().flatten() {
                let source = group["source"].as_str().unwrap_or_default();
                let target = group["target"].as_str().unwrap_or_default();
                for element in group["element"].as_array().into_iter().flatten() {
                    let code = element["code"].as_str().unwrap_or_default();
                    let slot = answers
                        .entry((
                            url.clone(),
                            source.to_owned(),
                            code.to_owned(),
                            target.to_owned(),
                        ))
                        .or_default();
                    for found in element["target"].as_array().into_iter().flatten() {
                        slot.push((
                            found["equivalence"].as_str().unwrap_or_default().to_owned(),
                            found["code"].as_str().unwrap_or_default().to_owned(),
                            found["display"].as_str().unwrap_or_default().to_owned(),
                        ));
                    }
                }
            }
        }
        Ok(Self { answers })
    }

    /// Mounts the stub on a new server and returns a client of it.
    pub(super) async fn serve(&self) -> Result<(MockServer, Client), Box<dyn Error>> {
        let server = MockServer::builder()
            .disable_request_recording()
            .start()
            .await;
        Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/r4/ConceptMap/$translate"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        let config = Config::new(format!("{}/r4", server.uri()).parse()?, WireVersion::R4)
            .with_timeout(Duration::from_secs(30))
            .with_retry(RetryPolicy {
                max_attempts: 1,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
            });
        Ok((server, Client::new(config)?))
    }
}

impl Respond for GuideTables {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let parameter = |name: &str| {
            body["parameter"]
                .as_array()
                .and_then(|parameters| {
                    parameters
                        .iter()
                        .find(|parameter| parameter["name"] == name)
                        .and_then(|parameter| {
                            ["valueUri", "valueCode", "valueString"]
                                .iter()
                                .find_map(|kind| parameter[*kind].as_str())
                        })
                })
                .unwrap_or_default()
                .to_owned()
        };
        let target = parameter("targetsystem");
        let key = (
            parameter("url"),
            parameter("system"),
            parameter("code"),
            target.clone(),
        );
        // A target with no code (an `unmatched` row) is no match, and a
        // target with no display carries none: FHIR R4 refuses an empty string.
        let matches: Vec<serde_json::Value> = self
            .answers
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|(_, code, _)| !code.is_empty())
            .map(|(equivalence, code, display)| {
                let mut coding = serde_json::json!({"system": target, "code": code});
                if !display.is_empty() {
                    coding["display"] = serde_json::Value::from(display.as_str());
                }
                serde_json::json!({"name": "match", "part": [
                    {"name": "equivalence", "valueCode": equivalence},
                    {"name": "concept", "valueCoding": coding},
                ]})
            })
            .collect();
        if matches.is_empty() {
            return terminology::translate_no_match("the table map holds no such code");
        }
        let mut parameter = vec![serde_json::json!({"name": "result", "valueBoolean": true})];
        parameter.extend(matches);
        let body = serde_json::json!({"resourceType": "Parameters", "parameter": parameter});
        ResponseTemplate::new(200)
            .insert_header("Content-Type", "application/fhir+json")
            .set_body_string(body.to_string())
    }
}
