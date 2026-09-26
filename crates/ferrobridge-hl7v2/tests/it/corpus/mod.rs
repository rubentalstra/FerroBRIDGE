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

mod compare;
mod sets;
mod tables;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

use bytes::BytesMut;
use ferrobridge_hl7v2::decode::Charset;
use ferrobridge_hl7v2::inbound::{self, Received};
use ferrobridge_hl7v2::map::{Mapped, Outcome, map};
use ferrobridge_hl7v2::mllp::Codec;
use ferrobridge_hl7v2::parse;
use ferrobridge_hl7v2::parse::structure::structure_for;
use ferrobridge_term::client::Client;
use ferrobridge_testkit::conformance::{Case, Corpus, record};
use fhir_types::codec::{Json, Value};
use tokio_util::codec::{Decoder, Encoder};

use crate::corpus::compare::{Targets, compare, expected_bundle};
use crate::corpus::sets::{
    Message, VENDOR, aira, converter, nist, normalised, reportstream, v2_to_fhir,
};
use crate::corpus::tables::GuideTables;
use crate::support;

/// The reason a case fails when the Bundle the map wrote does not decode.
const UNDECODABLE: &str = "the Bundle does not decode as R4";

/// The reason a case fails when a `message` Bundle does not open with a
/// `MessageHeader` (<https://hl7.org/fhir/R4/bundle.html#invs>, `bdl-12`).
const HEADERLESS: &str = "the message Bundle breaks bdl-12";

/// Whether `bundle` keeps `bdl-12`: when `Bundle.type` is `message`, the
/// first entry's resource is a `MessageHeader`.
fn keeps_bdl_12(bundle: &Value) -> bool {
    if bundle.get("type").and_then(Value::as_str) != Some("message") {
        return true;
    }
    bundle
        .get("entry")
        .and_then(Value::as_array)
        .and_then(<[Value]>::first)
        .and_then(|entry| entry.get("resource"))
        .and_then(|resource| resource.get("resourceType"))
        .and_then(Value::as_str)
        == Some("MessageHeader")
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

/// The run's counted outcomes, by kind, with one `supplemented:<map>` count
/// per supplement map the run used, so a verdict can be attributed to it.
fn counted(mapped: &Mapped) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = mapped
        .counts()
        .into_iter()
        .map(|(kind, count)| (String::from(kind), count))
        .collect();
    for outcome in mapped.outcomes() {
        if let Outcome::Supplemented { map, .. } = outcome {
            let slot = counts.entry(format!("supplemented:{map}")).or_insert(0);
            *slot = slot.saturating_add(1);
        }
    }
    counts
}

/// The message family and the version a message declares, read from its MSH
/// segment whatever the face answers, so a refused case counts too: MSH-9.1,
/// or the code before `_` in MSH-9.3 when MSH-9.1 is empty, and MSH-12.1.
fn declared(message: &[u8]) -> (Option<String>, Option<String>) {
    // NOTE: no specification governs this: our own design; a message with no
    // MSH segment to lex declares no family or version, and its case says why.
    let Ok(lexed) = parse::lex::lex(&String::from_utf8_lossy(message), Charset::Ascii) else {
        return (None, None);
    };
    let header = &lexed.message;
    let family = header
        .message_type(1)
        .filter(|code| parse::valued(code))
        .or_else(|| header.message_type(3)?.split('_').next())
        .filter(|code| parse::valued(code))
        .map(str::to_owned);
    let version = header
        .version()
        .filter(|version| parse::valued(version))
        .map(str::to_owned);
    (family, version)
}

/// Runs one message through the face and returns its verdict, with the
/// family and the version the message declares.
async fn run(
    message: &Message,
    guide: &ferrobridge_hl7v2::map::corpus::Corpus,
    targets: &Targets,
    client: &Client,
) -> Case {
    let normal = normalised(&message.bytes);
    let mut case = verdict(message, &normal, guide, targets, client).await;
    let (family, version) = declared(&normal);
    if let Some(family) = family {
        case = case.with_family(family);
    }
    if let Some(version) = version {
        case = case.with_version(version);
    }
    case
}

/// Runs one normalised message through the face and returns its verdict.
async fn verdict(
    message: &Message,
    normal: &[u8],
    guide: &ferrobridge_hl7v2::map::corpus::Corpus,
    targets: &Targets,
    client: &Client,
) -> Case {
    let bytes = match framed(normal) {
        Ok(bytes) => bytes,
        Err(reason) => return Case::fail(&message.id, reason),
    };
    let inbound = match inbound::receive(&bytes, Charset::Ascii, structure_for, support::STAMP) {
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
        return Case::fail(&message.id, format!("{UNDECODABLE}: {error}")).with_outcomes(outcomes);
    }
    if !keeps_bdl_12(mapped.bundle()) {
        return Case::fail(&message.id, HEADERLESS).with_outcomes(outcomes);
    }
    Case::pass(&message.id).with_outcomes(outcomes)
}

/// Runs every message through the guide with the crate's shipped supplements
/// over it, as the face runs it, and records the verdicts of `corpus`.
///
/// A difference from an expected Bundle is classified against the guide
/// alone ([`Targets`]), so a class names what the guide lacks whatever the
/// supplements fill.
async fn measure(corpus: Corpus, messages: &[Message]) -> Result<(), Box<dyn Error>> {
    let targets = Targets::of(&support::corpus());
    let shipped = support::shipped();
    let tables = GuideTables::load(&support::package())?;
    let (_server, client) = tables.serve().await?;
    let mut cases = Vec::with_capacity(messages.len());
    for message in messages {
        cases.push(run(message, &shipped, &targets, &client).await);
    }
    let outcome = record(corpus, &cases)?;
    assert!(
        outcome.regressed.is_empty(),
        "cases the {corpus} pass list records no longer pass: {:?}",
        outcome.regressed
    );
    let undecodable: Vec<(&str, &str)> = cases
        .iter()
        .filter_map(|case| Some((case.id(), case.failure()?)))
        .filter(|(_, failure)| failure.starts_with(UNDECODABLE))
        .collect();
    assert_eq!(
        undecodable,
        Vec::<(&str, &str)>::new(),
        "every Bundle the {corpus} corpus maps decodes as R4"
    );
    let headerless: Vec<&str> = cases
        .iter()
        .filter(|case| case.failure() == Some(HEADERLESS))
        .map(Case::id)
        .collect();
    assert_eq!(
        headerless,
        Vec::<&str>::new(),
        "every message Bundle the {corpus} corpus maps opens with its MessageHeader (bdl-12)"
    );
    let left_out: usize = cases
        .iter()
        .filter_map(|case| case.outcomes().get("undecodable"))
        .sum();
    assert_eq!(
        left_out, 0,
        "no resource of the {corpus} corpus is left out of its Bundle for failing to decode"
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
