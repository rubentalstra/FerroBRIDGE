// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 v2 message corpora, each message run from an MLLP frame to an R4
//! Bundle, with the verdicts reported through the testkit's conformance
//! instrument.
//!
//! The three vendored sets under `vendor/` (the Microsoft FHIR-Converter
//! samples, the CDC `ReportStream` data tests and the HL7 v2-to-FHIR benchmark
//! messages) are the `hl7v2` corpus. The NIST test bundles and a part of the
//! AIRA MQE examples, fetched at build time by
//! `scripts/vendor/hl7v2-samples.sh --build-time`, are the `hl7v2-smoke`
//! corpus, which skips with its reason when they are absent.
//!
//! A case passes when the message is framed, decoded, parsed with no refusal,
//! mapped, and the Bundle decodes as R4. An acknowledgment passes when it
//! parses, since the face answers it and maps none. Every outcome the run
//! counts is recorded beside the verdict, and where the set carries an
//! expected Bundle for the message, every difference from it is a counted
//! outcome that never decides the verdict: the expected Bundles come from
//! other converters, so they are a comparison and never the guide's answer.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use bytes::BytesMut;
use ferrobridge_hl7v2::decode::Charset;
use ferrobridge_hl7v2::inbound::{self, Received};
use ferrobridge_hl7v2::map::corpus::{Condition, Kind};
use ferrobridge_hl7v2::map::{Mapped, map};
use ferrobridge_hl7v2::mllp::Codec;
use ferrobridge_term::client::Client;
use ferrobridge_term::config::{Config, RetryPolicy, WireVersion};
use ferrobridge_testkit::conformance::{Case, Corpus, record};
use ferrobridge_testkit::stubs::terminology;
use fhir_types::codec::{Json, Value};
use tokio_util::codec::{Decoder, Encoder};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::support;

/// The vendored and fetched sets, relative to this crate's manifest.
const VENDOR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor");

/// The expected Bundles of the Microsoft set, under its vendored root.
const CONVERTER_EXPECTED: &str =
    "src/Microsoft.Health.Fhir.Liquid.Converter.FunctionalTests/TestData/Expected/Hl7v2";

/// The `ReportStream` data tests, under that set's vendored root.
const REPORTSTREAM_TESTS: &str = "prime-router/src/testIntegration/resources/datatests";

/// The `ReportStream` directories whose `.fhir` file is the conversion of the
/// `.hl7` message beside it; in `FHIR_to_HL7/` the direction is the reverse.
const REPORTSTREAM_CONVERSIONS: &[&str] = &["HL7_to_FHIR", "mappinginventory"];

/// How many messages of each AIRA example file the smoke corpus reads.
///
/// No specification governs this: our own design; the files hold 13,583
/// generated messages, and the first of each file keeps the case list short
/// and the same on every run.
const AIRA_PER_FILE: usize = 25;

/// One message of a corpus, with the Bundle its set expects for it.
struct Message {
    /// The case id: the set, then the path under it, spaces as `%20`.
    id: String,
    /// The bytes as the file holds them.
    bytes: Vec<u8>,
    /// The file holding the expected Bundle, when the set carries one.
    expected: Option<PathBuf>,
}

/// The case id of `relative` under `set`.
fn case_id(set: &str, relative: &Path) -> String {
    let path = relative.to_string_lossy().replace('\\', "/");
    format!("{set}/{path}").replace(' ', "%20")
}

/// Every file under `directory`, relative to `root`, in path order.
fn files(root: &Path, directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The Microsoft FHIR-Converter samples, each with the expected Bundle its
/// file name names.
fn converter() -> Result<Vec<Message>, Box<dyn Error>> {
    let root = Path::new(VENDOR).join("fhir-converter");
    let mut expected = BTreeMap::new();
    for path in files(&root, &root.join(CONVERTER_EXPECTED))? {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if let Some(stem) = name
            .as_deref()
            .and_then(|name| name.strip_suffix("-expected.json"))
        {
            expected.insert(stem.to_owned(), root.join(&path));
        }
    }
    let mut messages = Vec::new();
    for path in files(&root, &root.join("data/SampleData/Hl7v2"))? {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        messages.push(Message {
            id: case_id("fhir-converter", &path),
            bytes: fs::read(root.join(&path))?,
            expected: stem.and_then(|stem| expected.get(&stem).cloned()),
        });
    }
    Ok(messages)
}

/// The `ReportStream` `.hl7` files, each with the `.fhir` conversion beside it
/// in the directories that hold one.
fn reportstream() -> Result<Vec<Message>, Box<dyn Error>> {
    let root = Path::new(VENDOR).join("reportstream");
    let tests = root.join(REPORTSTREAM_TESTS);
    let mut messages = Vec::new();
    for path in files(&root, &tests)? {
        if path.extension().is_none_or(|extension| extension != "hl7") {
            continue;
        }
        let converts = root
            .join(&path)
            .strip_prefix(&tests)?
            .components()
            .next()
            .is_some_and(|first| {
                REPORTSTREAM_CONVERSIONS
                    .iter()
                    .any(|directory| first.as_os_str() == *directory)
            });
        let bundle = root.join(path.with_extension("fhir"));
        messages.push(Message {
            id: case_id("reportstream", &path),
            bytes: fs::read(root.join(&path))?,
            expected: (converts && bundle.is_file()).then_some(bundle),
        });
    }
    Ok(messages)
}

/// The seven benchmark messages derived from the v2-to-FHIR page and the
/// sample message beside its Bundle, each with the HL7-authored Bundle whose
/// file name names its structure.
fn v2_to_fhir() -> Result<Vec<Message>, Box<dyn Error>> {
    let root = Path::new(VENDOR).join("v2-to-fhir");
    let bundles = root.join("samples/fhir-bundles");
    // NOTE: `samples/fhir-bundles/FHIR_bundle.hl7_MDM_T02.json` holds XML at the
    // pin, so the MDM_T02 Bundle is read from its `.xml` twin.
    let expected_for = |structure: &str| match structure {
        "ADT_A01" => Some(bundles.join("FHIR_bundle.hl7_ADT_A01.json")),
        "MDM_T02" => Some(bundles.join("FHIR_bundle.hl7_MDM_T02.xml")),
        _ => None,
    };
    let mut messages = Vec::new();
    for path in files(&root, &root.join("derived"))? {
        let structure = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());
        messages.push(Message {
            id: case_id("v2-to-fhir", &path),
            bytes: fs::read(root.join(&path))?,
            expected: structure.as_deref().and_then(expected_for),
        });
    }
    let sample = Path::new("samples/messages/Message.hl7_MDM_T02.txt");
    messages.push(Message {
        id: case_id("v2-to-fhir", sample),
        bytes: fs::read(root.join(sample))?,
        expected: expected_for("MDM_T02"),
    });
    Ok(messages)
}

/// The NIST test steps: every `Message.txt` of the three fetched bundles.
fn nist(root: &Path) -> Result<Vec<Message>, Box<dyn Error>> {
    let mut messages = Vec::new();
    for path in files(root, root)? {
        if path.file_name().is_some_and(|name| name == "Message.txt") {
            messages.push(Message {
                id: case_id("nist", &path),
                bytes: fs::read(root.join(&path))?,
                expected: None,
            });
        }
    }
    Ok(messages)
}

/// The first [`AIRA_PER_FILE`] messages of each fetched AIRA example file,
/// split where a segment opens with `MSH`.
fn aira(root: &Path) -> Result<Vec<Message>, Box<dyn Error>> {
    let mut messages = Vec::new();
    for path in files(root, root)? {
        let bytes = fs::read(root.join(&path))?;
        let normal = normalised(&bytes);
        let mut starts: Vec<usize> = Vec::new();
        for (index, _) in normal.iter().enumerate() {
            let opens = index == 0 || normal.get(index.wrapping_sub(1)) == Some(&b'\r');
            if opens && normal.get(index..index.saturating_add(4)) == Some(b"MSH|".as_slice()) {
                starts.push(index);
            }
        }
        for (number, window) in starts.iter().enumerate().take(AIRA_PER_FILE) {
            let end = starts
                .get(number.saturating_add(1))
                .copied()
                .unwrap_or(normal.len());
            messages.push(Message {
                id: format!(
                    "{}#{}",
                    case_id("aira-mqe", &path),
                    number.saturating_add(1)
                ),
                bytes: normal.get(*window..end).unwrap_or_default().to_vec(),
                expected: None,
            });
        }
    }
    Ok(messages)
}

/// The message as a frame carries it: a leading UTF-8 byte order mark
/// removed, every line ending a carriage return, and no empty segment.
///
/// No specification governs this: our own design; the byte order mark and
/// the line endings are how a file stores the text, and HL7 v2.5.1 chapter 2
/// §2.5.4 ends every segment with a carriage return.
fn normalised(bytes: &[u8]) -> Vec<u8> {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut out = Vec::with_capacity(text.len());
    for line in text.split(|byte| *byte == b'\r' || *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        out.extend_from_slice(line);
        out.push(b'\r');
    }
    out
}

/// The message after one MLLP frame round trip, or why it cannot travel.
fn framed(message: &[u8]) -> Result<Vec<u8>, String> {
    let mut codec = Codec::default();
    let mut buffer = BytesMut::new();
    codec
        .encode(message, &mut buffer)
        .map_err(|error| format!("cannot be framed: {error}"))?;
    match codec.decode(&mut buffer) {
        Ok(Some(message)) => Ok(message),
        Ok(None) => Err(String::from("the frame does not complete")),
        Err(error) => Err(format!("the frame is refused: {error}")),
    }
}

/// The acknowledgment code and the first error text of an answer.
fn answer_reason(code: &str, reply: &[u8]) -> String {
    let text = String::from_utf8_lossy(reply);
    let error = text
        .split('\r')
        .find(|segment| segment.starts_with("ERR|"))
        .map(|segment| {
            let fields: Vec<&str> = segment.split('|').collect();
            let condition = fields
                .get(3)
                .and_then(|field| field.split('^').nth(1))
                .unwrap_or_default();
            let detail = fields.get(8).copied().unwrap_or_default();
            format!("{condition}: {detail}")
        })
        .unwrap_or_default();
    format!("answered {code}: {error}")
}

/// What the guide's maps target, to classify a difference from an expected
/// Bundle.
struct Targets {
    /// Per message map, by its structure name: each resource type its rows
    /// write, with whether every such row is narrative-gated.
    resources: BTreeMap<String, BTreeMap<String, bool>>,
    /// Per resource type and top-level element: whether every segment map row
    /// that writes it is narrative-gated.
    elements: BTreeMap<(String, String), bool>,
}

/// The leading name of a target such as `Patient[1]` or `identifier[2].value`.
fn head(target: &str) -> &str {
    target.split(['[', '.', '(']).next().unwrap_or(target)
}

impl Targets {
    fn of(guide: &ferrobridge_hl7v2::map::corpus::Corpus) -> Self {
        let mut resources: BTreeMap<String, BTreeMap<String, bool>> = BTreeMap::new();
        let mut elements: BTreeMap<(String, String), bool> = BTreeMap::new();
        for concept_map in guide.maps() {
            match concept_map.kind {
                Kind::Message => {
                    let Some(structure) = concept_map
                        .rows
                        .first()
                        .and_then(|row| row.source.split(['.', ':']).next())
                    else {
                        continue;
                    };
                    let written = resources.entry(structure.to_owned()).or_default();
                    for row in &concept_map.rows {
                        let narrative = row.condition == Condition::Narrative;
                        let every = written
                            .entry(head(&row.target_code).to_owned())
                            .or_insert(true);
                        *every = *every && narrative;
                    }
                }
                Kind::Segment => {
                    let Some((_, resource)) = concept_map.id.rsplit_once("-to-") else {
                        continue;
                    };
                    for row in &concept_map.rows {
                        let narrative = row.condition == Condition::Narrative;
                        let key = (
                            resource.to_ascii_lowercase(),
                            head(&row.target_code).to_owned(),
                        );
                        let every = elements.entry(key).or_insert(true);
                        *every = *every && narrative;
                    }
                }
                Kind::Datatype | Kind::Table | Kind::Other => {}
            }
        }
        Self {
            resources,
            elements,
        }
    }

    /// The class of a resource type the expected Bundle holds and the run
    /// does not.
    fn resource(&self, structure: &str, resource: &str) -> &'static str {
        match self
            .resources
            .get(structure)
            .and_then(|written| written.get(resource))
        {
            None => "corpus-gap",
            Some(true) => "supplement",
            Some(false) => "defect",
        }
    }

    /// The class of a top-level element the expected Bundle writes on a
    /// resource type and the run does not.
    fn element(&self, resource: &str, element: &str) -> &'static str {
        match self
            .elements
            .get(&(resource.to_ascii_lowercase(), element.to_owned()))
        {
            None => "corpus-gap",
            Some(true) => "supplement",
            Some(false) => "defect",
        }
    }
}

/// The resources of a Bundle by type: how many, and the union of their
/// top-level element names.
fn inventory(bundle: &Value) -> BTreeMap<String, (usize, BTreeSet<String>)> {
    let mut found: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    let entries = bundle
        .get("entry")
        .and_then(Value::as_array)
        .unwrap_or_default();
    for resource in entries.iter().filter_map(|entry| entry.get("resource")) {
        let (Some(kind), Some(object)) = (
            resource.get("resourceType").and_then(Value::as_str),
            resource.as_object(),
        ) else {
            continue;
        };
        let slot = found.entry(kind.to_owned()).or_default();
        slot.0 = slot.0.saturating_add(1);
        slot.1.extend(
            object
                .keys()
                .filter(|key| !matches!(key.as_str(), "resourceType" | "id" | "meta"))
                .cloned(),
        );
    }
    found
}

/// Reads an expected Bundle, JSON or XML by its extension.
fn expected_bundle(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    if path.extension().is_some_and(|extension| extension == "xml") {
        fhir_types::xml::from_xml(&fhir_types::r4::schema::SCHEMAS, text)
            .map(Value::Object)
            .map_err(|error| error.to_string())
    } else {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }
}

/// Counts every difference between the run's Bundle and the expected one,
/// resource type by resource type and element by element, each classified:
/// `corpus-gap` where no map of the guide writes it, `supplement` (#256)
/// where every row that writes it is narrative-gated, `defect` where a row
/// writes it and the run did not, and `beyond-oracle` where the run writes
/// what the expected Bundle leaves out.
fn compare(
    ours: &Value,
    expected: &Value,
    structure: &str,
    targets: &Targets,
    outcomes: &mut BTreeMap<String, usize>,
) {
    let ours = inventory(ours);
    let theirs = inventory(expected);
    let mut count = |key: String| {
        let slot = outcomes.entry(key).or_insert(0);
        *slot = slot.saturating_add(1);
    };
    for (kind, (expected_count, expected_elements)) in &theirs {
        let Some((our_count, our_elements)) = ours.get(kind) else {
            count(format!(
                "compare:{}:resource:{kind}",
                targets.resource(structure, kind)
            ));
            continue;
        };
        if our_count != expected_count {
            count(format!("compare:count:{kind}"));
        }
        for element in expected_elements.difference(our_elements) {
            count(format!(
                "compare:{}:element:{kind}.{element}",
                targets.element(kind, element)
            ));
        }
        for element in our_elements.difference(expected_elements) {
            count(format!("compare:beyond-oracle:element:{kind}.{element}"));
        }
    }
    for kind in ours.keys().filter(|kind| !theirs.contains_key(*kind)) {
        count(format!("compare:beyond-oracle:resource:{kind}"));
    }
}

/// A `$translate` question: the table map's url, the source system, the code
/// and the target system.
type Question = (String, String, String, String);

/// One target of a table map row: its equivalence, code and display.
type Answer = (String, String, String);

/// A terminology server loaded with the guide's own table maps: it answers
/// `$translate` from the `ConceptMap` the request names, by source system,
/// code and target system, as a server holding those maps does.
#[derive(Debug, Clone, Default)]
struct GuideTables {
    answers: BTreeMap<Question, Vec<Answer>>,
}

impl GuideTables {
    /// Reads every `ConceptMap-table-*.json` of the package.
    fn load(package: &Path) -> Result<Self, Box<dyn Error>> {
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
    async fn serve(&self) -> Result<(MockServer, Client), Box<dyn Error>> {
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

/// An error and every cause under it, as one line.
fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// The run's counted outcomes, by kind.
fn counted(mapped: &Mapped) -> BTreeMap<String, usize> {
    mapped
        .counts()
        .into_iter()
        .map(|(kind, count)| (String::from(kind), count))
        .collect()
}

/// Runs one message through the face and returns its verdict.
async fn run(
    message: &Message,
    guide: &ferrobridge_hl7v2::map::corpus::Corpus,
    targets: &Targets,
    client: &Client,
) -> Case {
    let bytes = match framed(&normalised(&message.bytes)) {
        Ok(bytes) => bytes,
        Err(reason) => return Case::fail(&message.id, reason),
    };
    let inbound = match inbound::receive(&bytes, Charset::Ascii, support::select, support::STAMP) {
        Received::Parsed(inbound) => inbound,
        Received::Answered { code, reply } => {
            return Case::fail(&message.id, answer_reason(&format!("{code:?}"), &reply));
        }
        Received::Unanswerable => {
            return Case::fail(&message.id, "unanswerable: no MSH header can be read");
        }
    };
    let parsed = inbound.parsed();
    if parsed.message().message_type(1) == Some("ACK") {
        let mut outcomes = BTreeMap::new();
        outcomes.insert(String::from("acknowledgment-not-mapped"), 1);
        return Case::pass(&message.id).with_outcomes(outcomes);
    }
    let mapped = match map(parsed, guide, Some(client)).await {
        Ok(mapped) => mapped,
        Err(error) => {
            return Case::fail(
                &message.id,
                format!("the map refuses it: {}", chain(&error)),
            );
        }
    };
    let mut outcomes = counted(&mapped);
    if let Some(path) = &message.expected {
        match expected_bundle(path) {
            Ok(expected) => compare(
                mapped.bundle(),
                &expected,
                parsed.structure_name(),
                targets,
                &mut outcomes,
            ),
            Err(_) => {
                outcomes.insert(String::from("compare:unreadable-expected"), 1);
            }
        }
    }
    let Some(object) = mapped.bundle().as_object() else {
        return Case::fail(&message.id, "the map wrote no Bundle object").with_outcomes(outcomes);
    };
    if let Err(error) = fhir_types::r4::bundle::Bundle::from_json(
        object,
        &mut fhir_types::codec::Path::root("Bundle"),
    ) {
        return Case::fail(
            &message.id,
            format!("the Bundle does not decode as R4: {error}"),
        )
        .with_outcomes(outcomes);
    }
    Case::pass(&message.id).with_outcomes(outcomes)
}

/// Runs every message and records the verdicts of `corpus`.
async fn measure(corpus: Corpus, messages: &[Message]) -> Result<(), Box<dyn Error>> {
    let guide = support::corpus();
    let targets = Targets::of(&guide);
    let tables = GuideTables::load(&support::package())?;
    let (_server, client) = tables.serve().await?;
    let mut cases = Vec::with_capacity(messages.len());
    for message in messages {
        cases.push(run(message, &guide, &targets, &client).await);
    }
    let outcome = record(corpus, &cases)?;
    assert!(
        outcome.regressed.is_empty(),
        "cases the {corpus} pass list records no longer pass: {:?}",
        outcome.regressed
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn conformance_the_vendored_hl7v2_corpora_hold_their_pass_list() -> Result<(), Box<dyn Error>>
{
    let mut messages = converter()?;
    messages.extend(reportstream()?);
    messages.extend(v2_to_fhir()?);
    measure(Corpus::Hl7v2, &messages).await
}

/// Whether the build-time sets are on disk, or why the smoke corpus skips.
#[expect(
    clippy::print_stderr,
    reason = "a corpus whose build-time sets are absent says it skipped"
)]
fn fetched(nist: &Path, aira: &Path) -> bool {
    let present = ["lri-r2", "loi-r1", "ss-r2"]
        .iter()
        .all(|branch| nist.join(branch).join("src").is_dir())
        && aira.join("examples").is_dir();
    if !present {
        eprintln!(
            "skipped: the NIST and AIRA sets are fetched at build time; run scripts/vendor/hl7v2-samples.sh --build-time"
        );
    }
    present
}

#[tokio::test(flavor = "multi_thread")]
async fn conformance_the_fetched_hl7v2_corpora_hold_their_pass_list() -> Result<(), Box<dyn Error>>
{
    let nist_root = Path::new(VENDOR).join("nist");
    let aira_root = Path::new(VENDOR).join("aira-mqe");
    if !fetched(&nist_root, &aira_root) {
        return Ok(());
    }
    let mut messages = nist(&nist_root)?;
    messages.extend(aira(&aira_root)?);
    measure(Corpus::Hl7v2Smoke, &messages).await
}

#[test]
fn normalising_removes_the_byte_order_mark_and_ends_every_segment_with_a_carriage_return() {
    let stored = b"\xEF\xBB\xBFMSH|^~\\&|A\r\nPID|1\n\nOBX|1\r\n";
    assert_eq!(normalised(stored), b"MSH|^~\\&|A\rPID|1\rOBX|1\r".to_vec());
}

#[test]
fn a_difference_the_guide_maps_is_a_defect_and_one_it_never_maps_a_corpus_gap() {
    let guide = support::corpus();
    let targets = Targets::of(&guide);
    assert_eq!(targets.resource("ORU_R01", "Patient"), "defect");
    assert_eq!(targets.resource("ORU_R01", "Immunization"), "corpus-gap");
    assert_eq!(targets.element("Patient", "name"), "defect");
    assert_eq!(targets.element("Patient", "photo"), "corpus-gap");
}
