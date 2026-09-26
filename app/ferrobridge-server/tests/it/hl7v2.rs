// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 v2 face over MLLP: a framed message in, the acknowledgment and
//! the CDR calls out.
//!
//! Every case starts the listener on an ephemeral port over a `wiremock` CDR
//! and a `wiremock` terminology server, sends a synthetic ORU^R01 from a plain
//! `TcpStream`, and asserts the answer (HL7 v2.5.1 chapter 2 §2.9.2.2 for the
//! codes) and what the CDR received. The mapping set is one synthetic
//! Observation context under `hl7v2_fixtures/`, which the face's configured
//! profile points the message's Observation at. Behind `FERROBRIDGE_E2E` the
//! same message goes into the reference CDR.

use crate::facade::EHR_ID;
use crate::facade::Echo;
use crate::facade::client;
use crate::facade::ehr_body;
use crate::facade::handle;
use crate::facade::mount_echo;
use crate::facade::settings;
use ferrobridge_hl7v2::decode::Charset;
use ferrobridge_hl7v2::mllp::Timeouts;
use ferrobridge_server::config::Hl7v2Settings;
use ferrobridge_server::facade::Facade;
use ferrobridge_server::facade::identity::store::MemoryStore;
use ferrobridge_server::facade::ingest::UnmappedEntries;
use ferrobridge_server::facade::programs;
use ferrobridge_server::hl7v2::Face;
use ferrobridge_server::hl7v2::Listener;
use ferrobridge_server::hl7v2::Listening;
use ferrobridge_testkit::stubs::terminology;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::Request;
use wiremock::Respond;
use wiremock::ResponseTemplate;
use wiremock::matchers;

/// The mapping set the face writes through: one Observation context.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/hl7v2_fixtures");

/// The profile the Observation context claims.
const PROFILE: &str = "http://example.org/fhir/StructureDefinition/ferrobridge-hl7v2-observation";

/// The containers the stub CDR hands out, in commit order.
const CONTAINERS: &[&str] = &[
    "3c9d6a70-0000-4000-8000-00000000001e",
    "3c9d6a70-0000-4000-8000-00000000001f",
];

/// The control id of the synthetic message.
const CONTROL_ID: &str = "MSG-V2-0001";

/// A laboratory result, ORU^R01 from a 2.5.1 sender with an empty MSH-18: one
/// patient, one order and one numeric result, every value invented for the
/// test.
fn oru_r01(control_id: &str) -> Vec<u8> {
    let segments = [
        format!(
            "MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|{control_id}|P|2.5.1"
        ),
        String::from(
            "PID|1||synthetic-subject-0001^^^NORTHLAB^MR||Synthetic^Sam^^^^^L||19800101|M",
        ),
        String::from("ORC|RE|PLC-1|FIL-1"),
        String::from("OBR|1|PLC-1|FIL-1|2345-7^Glucose^LN|||20260925120000+0200"),
        String::from(
            "OBX|1|NM|2345-7^Glucose^LN^^^^^^Synthetic glucose||5.4|mmol/L^mmol/L^UCUM|3.9-5.8|N|||F|||20260925130000+0200",
        ),
    ];
    let mut bytes = Vec::new();
    for segment in segments {
        bytes.extend_from_slice(segment.as_bytes());
        bytes.push(b'\r');
    }
    bytes
}

/// Wraps a message in the MLLP envelope.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x0B];
    bytes.extend_from_slice(message);
    bytes.extend_from_slice(&[0x1C, 0x0D]);
    bytes
}

/// Reads one framed acknowledgment and returns its text.
async fn read_ack(stream: &mut TcpStream) -> Result<String, Box<dyn StdError>> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut byte)).await??;
        if read == 0 {
            return Err("the connection closed before the acknowledgment ended".into());
        }
        buffer.extend_from_slice(&byte);
        if buffer.ends_with(&[0x1C, 0x0D]) {
            break;
        }
    }
    let inner = buffer
        .get(1..buffer.len().saturating_sub(2))
        .ok_or("a framed answer")?;
    Ok(String::from_utf8(inner.to_vec())?)
}

/// The segments of an acknowledgment that open with `id`.
fn segments<'a>(ack: &'a str, id: &str) -> Vec<&'a str> {
    ack.split('\r')
        .filter(|segment| segment.starts_with(&format!("{id}|")))
        .collect()
}

/// A terminology server loaded with the one table value the Observation
/// needs: OBX-11 `F` through `table-hl70085-to-observation-status`. Every
/// other translation states no match.
#[derive(Debug, Clone, Copy)]
struct Tables;

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
        let url = parameter("url", "valueUri");
        let code = parameter("code", "valueCode");
        if url.ends_with("/table-hl70085-to-observation-status") && code == "F" {
            return terminology::translate(
                "equivalent",
                "http://hl7.org/fhir/observation-status",
                "final",
                "Final",
            );
        }
        terminology::translate_no_match("no mapping for the code")
    }
}

/// Returns a terminology client over a started stub.
async fn terminology_stub() -> (MockServer, ferrobridge_term::client::Client) {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/r4/ConceptMap/$translate"))
        .respond_with(Tables)
        .mount(&server)
        .await;
    let base = format!("{}/r4", server.uri()).parse().expect("a base url");
    let config =
        ferrobridge_term::config::Config::new(base, ferrobridge_term::config::WireVersion::R4)
            .with_timeout(Duration::from_secs(5))
            .with_retry(ferrobridge_term::config::RetryPolicy {
                max_attempts: 1,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
            });
    let client = ferrobridge_term::client::Client::new(config).expect("a terminology client");
    (server, client)
}

/// The vendored v2-to-FHIR package.
fn package() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/package")
}

/// Returns the face's settings, claiming [`PROFILE`] for an Observation when
/// `claim` is set.
fn face_settings(claim: bool) -> Hl7v2Settings {
    let profiles = if claim {
        BTreeMap::from([(String::from("Observation"), String::from(PROFILE))])
    } else {
        BTreeMap::new()
    };
    Hl7v2Settings {
        listen: "127.0.0.1:0".parse().expect("a socket address"),
        default_charset: Charset::Ascii,
        concept_maps: package(),
        supplements: Vec::new(),
        unmapped: UnmappedEntries::SkipAndCount,
        ehr_policy: None,
        profiles,
        timeouts: Timeouts::default(),
        frame_limit: 1024 * 1024,
        log_outcomes: true,
        senders: Vec::new(),
    }
}

/// A face serving on an ephemeral port, with its terminology stub.
struct Serving {
    /// The stub terminology server, held so it keeps answering.
    _terminology: MockServer,
    /// Where the listener accepts.
    address: SocketAddr,
    /// Whether the listener reads up.
    listening: Listening,
    /// The switch that stops it.
    stop: Option<oneshot::Sender<()>>,
    /// The listener task.
    task: JoinHandle<std::io::Result<()>>,
}

impl Serving {
    /// Starts the face over `facade` under `lane`.
    async fn start(facade: Arc<Facade>, lane: Hl7v2Settings) -> Result<Self, Box<dyn StdError>> {
        let (terminology_server, terminology) = terminology_stub().await;
        let corpus = ferrobridge_server::hl7v2::load_corpus(&lane)?;
        let face = Arc::new(Face::new(facade, corpus, Some(terminology), &lane));
        let socket = TcpListener::bind("127.0.0.1:0").await?;
        let address = socket.local_addr()?;
        let listening = Listening::default();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(ferrobridge_server::hl7v2::serve_until(
            socket,
            face,
            Listener::of(&lane, Duration::from_secs(10)),
            listening.clone(),
            async {
                let _signal = stopped.await;
            },
        ));
        Ok(Self {
            _terminology: terminology_server,
            address,
            listening,
            stop: Some(stop),
            task,
        })
    }

    /// Sends one framed message on a new connection and reads the answer.
    async fn send(&self, message: &[u8]) -> Result<String, Box<dyn StdError>> {
        let mut stream = TcpStream::connect(self.address).await?;
        stream.write_all(&frame(message)).await?;
        read_ack(&mut stream).await
    }

    /// Signals the stop without waiting for the listener to end.
    fn signal(&mut self) -> Result<(), Box<dyn StdError>> {
        if let Some(stop) = self.stop.take() {
            stop.send(()).map_err(|()| "the listener is gone")?;
        }
        Ok(())
    }

    /// Signals the stop and waits for the listener to end.
    async fn stop(mut self) -> Result<(), Box<dyn StdError>> {
        self.signal()?;
        tokio::time::timeout(Duration::from_secs(10), self.task).await???;
        Ok(())
    }
}

/// A running face over stub upstreams.
struct Running {
    /// The stub CDR.
    cdr: MockServer,
    /// The face.
    serving: Serving,
}

impl Running {
    /// Starts the face whose settings claim the profile when `claim` is set,
    /// over a CDR whose EHR lookup answers after `delay`.
    async fn start(claim: bool, delay: Duration) -> Result<Self, Box<dyn StdError>> {
        let lookup = ResponseTemplate::new(200)
            .set_body_string(ehr_body())
            .set_delay(delay);
        Self::start_with(face_settings(claim), lookup).await
    }

    /// Starts the face under `lane` over a CDR whose EHR lookup answers
    /// `lookup`.
    async fn start_with(
        lane: Hl7v2Settings,
        lookup: ResponseTemplate,
    ) -> Result<Self, Box<dyn StdError>> {
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
        Mock::given(matchers::method("GET"))
            .and(matchers::path("/ehr"))
            .respond_with(lookup)
            .mount(&cdr)
            .await;
        mount_echo(&cdr, CONTAINERS, Echo::Sent).await;
        let set = programs::read_set(std::path::Path::new(FIXTURES))?;
        let templates = programs::fetch_templates(&set, &client(&cdr)).await?;
        let compiled = programs::compile_set(&set, &templates)?;
        let store = Arc::new(MemoryStore::new());
        let facade = Arc::new(Facade::new(
            compiled,
            handle(&store),
            client(&cdr),
            settings(),
        ));
        let serving = Serving::start(facade, lane).await?;
        Ok(Self { cdr, serving })
    }

    /// Sends one framed message on a new connection and reads the answer.
    async fn send(&self, message: &[u8]) -> Result<String, Box<dyn StdError>> {
        self.serving.send(message).await
    }

    /// Counts the contribution commits the CDR received.
    async fn contributions(&self) -> usize {
        self.requests("POST", &format!("/ehr/{EHR_ID}/contribution"))
            .await
            .len()
    }

    /// Returns the bodies of the requests the CDR received on `path`.
    async fn requests(&self, method: &str, path: &str) -> Vec<Vec<u8>> {
        self.cdr
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|request| request.method.as_str() == method && request.url.path() == path)
            .map(|request| request.body)
            .collect()
    }

    /// Signals the stop and waits for the listener to end.
    async fn stop(self) -> Result<(), Box<dyn StdError>> {
        self.serving.stop().await
    }
}

#[tokio::test]
async fn a_mapped_message_commits_one_contribution_and_is_answered_aa()
-> Result<(), Box<dyn StdError>> {
    let running = Running::start(true, Duration::ZERO).await?;
    let ack = running.send(&oru_r01(CONTROL_ID)).await?;
    assert!(
        running.serving.listening.is_up(),
        "the listener reads up while serving"
    );
    assert_eq!(
        vec![format!("MSA|AA|{CONTROL_ID}")],
        segments(&ack, "MSA"),
        "{ack}"
    );
    assert!(segments(&ack, "ERR").is_empty(), "{ack}");
    assert!(
        ack.starts_with("MSH|^~\\&|EHR|SOUTHCLINIC|LAB|NORTHLAB|"),
        "the answer swaps the applications: {ack}"
    );
    assert_eq!(1, running.contributions().await, "one commit");
    let committed = running
        .requests("POST", &format!("/ehr/{EHR_ID}/contribution"))
        .await;
    let body = String::from_utf8(committed.first().cloned().unwrap_or_default())?;
    assert!(
        body.contains(CONTROL_ID) && body.contains("ORU^R01"),
        "the FEEDER_AUDIT names the message by MSH-10 and its type"
    );
    running.stop().await
}

#[tokio::test]
async fn a_message_sent_again_is_answered_aa_and_commits_nothing() -> Result<(), Box<dyn StdError>>
{
    let running = Running::start(true, Duration::ZERO).await?;
    let first = running.send(&oru_r01(CONTROL_ID)).await?;
    assert_eq!(
        vec![format!("MSA|AA|{CONTROL_ID}")],
        segments(&first, "MSA"),
        "{first}"
    );
    let again = running.send(&oru_r01(CONTROL_ID)).await?;
    assert_eq!(
        vec![format!("MSA|AA|{CONTROL_ID}")],
        segments(&again, "MSA"),
        "a replay answers as the first delivery did: {again}"
    );
    assert_eq!(
        1,
        running.contributions().await,
        "the replay commits nothing"
    );
    running.stop().await
}

#[tokio::test]
async fn a_message_no_program_maps_is_answered_ae_naming_each_entry()
-> Result<(), Box<dyn StdError>> {
    let running = Running::start(false, Duration::ZERO).await?;
    let ack = running.send(&oru_r01(CONTROL_ID)).await?;
    assert_eq!(
        vec![format!("MSA|AE|{CONTROL_ID}")],
        segments(&ack, "MSA"),
        "{ack}"
    );
    let errors = segments(&ack, "ERR");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("carries no entry this server maps")),
        "{ack}"
    );
    assert!(
        errors.iter().any(|error| error.contains("urn:uuid:")
            && error.contains("Observation")
            && error.contains("claims one of the profiles")),
        "the Observation is named with why it was skipped: {ack}"
    );
    assert!(
        errors
            .iter()
            .all(|error| error.contains("|207^Application error^HL70357|E|")),
        "{ack}"
    );
    assert_eq!(0, running.contributions().await, "nothing is committed");
    running.stop().await
}

#[tokio::test]
async fn a_sending_facility_not_listed_is_answered_ar_at_msh_4_and_never_mapped()
-> Result<(), Box<dyn StdError>> {
    let mut lane = face_settings(true);
    lane.senders = vec![ferrobridge_server::hl7v2::Sender::Namespace(String::from(
        "SOUTHLAB",
    ))];
    let lookup = ResponseTemplate::new(200).set_body_string(ehr_body());
    let running = Running::start_with(lane, lookup).await?;
    let ack = running.send(&oru_r01(CONTROL_ID)).await?;
    assert_eq!(
        vec![format!("MSA|AR|{CONTROL_ID}")],
        segments(&ack, "MSA"),
        "{ack}"
    );
    let errors = segments(&ack, "ERR");
    assert!(
        errors
            .iter()
            .any(|error| error.starts_with("ERR||MSH^1^4|207^Application error^HL70357|E|")),
        "{ack}"
    );
    assert!(
        running.requests("GET", "/ehr").await.is_empty(),
        "a refused sender's message never reaches the ingest"
    );
    assert_eq!(0, running.contributions().await);
    running.stop().await
}

#[tokio::test]
async fn a_listed_sending_facility_is_mapped() -> Result<(), Box<dyn StdError>> {
    let mut lane = face_settings(true);
    lane.senders = vec![ferrobridge_server::hl7v2::Sender::Namespace(String::from(
        "NORTHLAB",
    ))];
    let lookup = ResponseTemplate::new(200).set_body_string(ehr_body());
    let running = Running::start_with(lane, lookup).await?;
    let ack = running.send(&oru_r01(CONTROL_ID)).await?;
    assert_eq!(
        vec![format!("MSA|AA|{CONTROL_ID}")],
        segments(&ack, "MSA"),
        "{ack}"
    );
    assert_eq!(1, running.contributions().await);
    running.stop().await
}

#[tokio::test]
async fn a_cdr_that_fails_is_answered_ar_so_the_sender_resends() -> Result<(), Box<dyn StdError>> {
    let running = Running::start_with(face_settings(true), ResponseTemplate::new(503)).await?;
    let ack = running.send(&oru_r01(CONTROL_ID)).await?;
    assert_eq!(
        vec![format!("MSA|AR|{CONTROL_ID}")],
        segments(&ack, "MSA"),
        "a failure unrelated to content is AR (chapter 2 §2.9.2.2): {ack}"
    );
    assert!(!segments(&ack, "ERR").is_empty(), "{ack}");
    assert_eq!(0, running.contributions().await, "nothing is committed");
    running.stop().await
}

#[tokio::test]
async fn a_message_in_flight_when_the_stop_arrives_is_committed_and_answered()
-> Result<(), Box<dyn StdError>> {
    let Running { cdr, mut serving } = Running::start(true, Duration::from_millis(500)).await?;
    let mut stream = TcpStream::connect(serving.address).await?;
    stream.write_all(&frame(&oru_r01(CONTROL_ID))).await?;
    let looked_up = || async {
        cdr.received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|request| request.method.as_str() == "GET" && request.url.path() == "/ehr")
    };
    for _ in 0..200 {
        if looked_up().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        looked_up().await,
        "the message reached the ingest before the stop"
    );
    let listening = serving.listening.clone();
    serving.signal()?;
    let ack = read_ack(&mut stream).await?;
    assert_eq!(
        vec![format!("MSA|AA|{CONTROL_ID}")],
        segments(&ack, "MSA"),
        "{ack}"
    );
    serving.stop().await?;
    let contributions = cdr
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| {
            request.method.as_str() == "POST"
                && request.url.path() == format!("/ehr/{EHR_ID}/contribution")
        })
        .count();
    assert_eq!(1, contributions, "the message in flight committed");
    assert!(!listening.is_up(), "the listener reads down after the stop");
    Ok(())
}

#[tokio::test]
async fn an_oru_r01_over_mllp_commits_a_composition_into_a_real_cdr()
-> Result<(), Box<dyn StdError>> {
    if !ferrobridge_testkit::containers::e2e_enabled() {
        return Ok(());
    }
    // NOTE: no specification governs this: our own design; no published
    // context maps the guide's resources in-tree (#258), so the suite's own context stands in.
    let cdr = ferrobridge_testkit::containers::cdr().await?;
    crate::facade_e2e::upload_template(cdr.base_url()).await?;
    let client = ferrobridge_server::cdr::CdrClient::new(
        &ferrobridge_server::cdr::config::CdrConfig::new(cdr.base_url().parse()?),
    )?;
    let set = programs::read_set(std::path::Path::new(FIXTURES))?;
    let templates = programs::fetch_templates(&set, &client).await?;
    let compiled = programs::compile_set(&set, &templates)?;
    let mut facade_settings = settings();
    facade_settings.ehr_policy = ferrobridge_server::facade::ehr::Policy::CreateOnFirstWrite;
    let facade = Arc::new(Facade::new(
        compiled,
        Arc::new(MemoryStore::new()),
        client,
        facade_settings,
    ));
    let serving = Serving::start(facade, face_settings(true)).await?;
    let control = "MSG-V2-E2E-0001";
    let ack = serving.send(&oru_r01(control)).await?;
    assert_eq!(
        vec![format!("MSA|AA|{control}")],
        segments(&ack, "MSA"),
        "the reference CDR committed the message: {ack}"
    );
    let again = serving.send(&oru_r01(control)).await?;
    assert_eq!(
        vec![format!("MSA|AA|{control}")],
        segments(&again, "MSA"),
        "a replay answers as the commit did: {again}"
    );
    serving.stop().await
}
