// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared helpers: the corpus, a parse shortcut, the test face behind the
//! MLLP listener, and the terminology stub.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrobridge_hl7v2::ack::{self, Code, Stamp};
use ferrobridge_hl7v2::decode::{self, Charset};
use ferrobridge_hl7v2::inbound::{self, Received};
use ferrobridge_hl7v2::map::corpus::Corpus;
use ferrobridge_hl7v2::mllp::{Handler, Malformed};
use ferrobridge_hl7v2::parse::{self, Parsed};
use ferrobridge_term::client::Client;
use ferrobridge_term::config::{Config, RetryPolicy, WireVersion};
use ferrobridge_testkit::stubs::terminology;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// The acknowledgment stamp every test answer carries.
pub(crate) const STAMP: Stamp<'static> = Stamp {
    control_id: "ACK00001",
    timestamp: "20260925143001+0200",
};

/// Decodes, lexes and groups a fixture, with ASCII as the agreed default.
pub(crate) fn parsed(bytes: &[u8]) -> Parsed {
    let decoded = decode::decode(bytes, Charset::Ascii).expect("the fixture decodes");
    let lexed = parse::lex(&decoded.text, decoded.charset).expect("the fixture lexes");
    let structure = parse::structure_for(&lexed.message).expect("the fixture names a structure");
    parse::group(lexed, structure)
}

/// The vendored v2-to-FHIR package.
pub(crate) fn package() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/package")
}

/// The corpus, loaded once per test.
pub(crate) fn corpus() -> Corpus {
    Corpus::load(&package()).expect("the vendored package loads")
}

/// The face the wire tests put behind the listener: it receives each message
/// and answers `AA` for one that parses with no refusal.
#[derive(Debug, Default)]
pub(crate) struct Face;

impl Handler for Face {
    fn handle(&self, message: Vec<u8>) -> impl Future<Output = Option<Vec<u8>>> + Send {
        let reply = match inbound::receive(&message, Charset::Ascii, parse::structure_for, STAMP) {
            Received::Parsed(inbound) => inbound.answer(Code::Accept, &[], STAMP).ok(),
            Received::Answered { reply, .. } => Some(reply),
            Received::Unanswerable => None,
        };
        std::future::ready(reply)
    }

    fn malformed(&self, kind: Malformed, partial: &[u8]) -> Option<Vec<u8>> {
        ack::reject_frame(partial, &kind.to_string(), STAMP)
    }
}

/// A terminology server loaded with a few of the guide's table maps: it
/// answers `$translate` for the `(concept map, code)` pairs it knows and
/// states no match for any other.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tables {
    answers: BTreeMap<(String, String), (String, String, String)>,
    asked: Arc<Mutex<Vec<(String, String)>>>,
}

impl Tables {
    /// Adds the answer to `code` through the table map `id`.
    pub(crate) fn answer(
        mut self,
        id: &str,
        code: &str,
        system: &str,
        target: &str,
        display: &str,
    ) -> Self {
        self.answers.insert(
            (
                format!("http://hl7.org/fhir/uv/v2mappings/ConceptMap/{id}"),
                String::from(code),
            ),
            (
                String::from(system),
                String::from(target),
                String::from(display),
            ),
        );
        self
    }

    /// Returns every `(concept map, code)` asked, in order.
    pub(crate) fn asked(&self) -> Vec<(String, String)> {
        self.asked.lock().expect("the lock is not poisoned").clone()
    }

    /// Mounts the stub on a new server and returns a client of it.
    pub(crate) async fn serve(&self) -> (MockServer, Client) {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/r4/ConceptMap/$translate"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        let base = format!("{}/r4", server.uri()).parse().expect("a base url");
        let config = Config::new(base, WireVersion::R4)
            .with_timeout(Duration::from_secs(5))
            .with_retry(RetryPolicy {
                max_attempts: 1,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
            });
        let client = Client::new(config).expect("a client");
        (server, client)
    }
}

impl Respond for Tables {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let parameter = |name: &str, kind: &str| {
            body["parameter"]
                .as_array()
                .and_then(|parameters| {
                    parameters
                        .iter()
                        .find(|parameter| parameter["name"] == name)
                        .and_then(|parameter| parameter[kind].as_str())
                })
                .map(String::from)
                .unwrap_or_default()
        };
        let key = (parameter("url", "valueUri"), parameter("code", "valueCode"));
        self.asked
            .lock()
            .expect("the lock is not poisoned")
            .push(key.clone());
        match self.answers.get(&key) {
            Some((system, code, display)) => {
                terminology::translate("equivalent", system, code, display)
            }
            None => terminology::translate_no_match("no mapping for the code"),
        }
    }
}
