// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 v2 face: an MLLP listener whose messages enter the facade's ingest
//! service.
//!
//! Each frame runs through `ferrobridge_hl7v2::inbound` (decode, parse,
//! group) and `ferrobridge_hl7v2::map` (the v2-to-FHIR `ConceptMaps`) into
//! an R4 message `Bundle`. The Bundle's entries then go through
//! [`crate::facade::ingest::Ingest::ingest_bundle`] as one transaction: the
//! programs are selected as the facade selects them, every composition's
//! `FEEDER_AUDIT` names the message by MSH-10 and its type, and the rule for
//! an entry no program maps is the configured one. The acknowledgment is
//! written only once the ingest settled, so `AA` means the CDR holds the
//! message's compositions ([`answer`]). No specification governs the
//! hand-off from a message Bundle to a transaction: our own design.
//!
//! No byte of a message reaches a log line. The span of a message names its
//! MSH-10 and its type, and the events name the acknowledgment code and
//! counts.

pub mod answer;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ferrobridge_hl7v2::ack::{self, Code, Header, Stamp};
use ferrobridge_hl7v2::decode::Charset;
use ferrobridge_hl7v2::inbound::{self, Inbound, Received};
use ferrobridge_hl7v2::map::corpus::{Corpus, CorpusError};
use ferrobridge_hl7v2::mllp::{self, Codec, Connection, Handler, Malformed, Timeouts};
use ferrobridge_hl7v2::parse::structure::structure_for;
use ferrobridge_hl7v2::parse::{self, Message};
use fhir_types::codec::Value;
use fhirconnect::engine::origin::{MessageControlId, MessageType, SourceItem};
use tokio::net::TcpListener;
use tracing::Instrument;

use crate::config::Hl7v2Settings;
use crate::facade::Facade;
use crate::facade::ingest::{Provenance, UnmappedEntries};
use crate::hl7v2::answer::{Settled, Tally};

/// Loads the guide's `ConceptMaps`, the supplements `ferrobridge-hl7v2`
/// ships, and every supplement directory of the settings over them, in order.
///
/// # Errors
///
/// Returns the [`CorpusError`] of the first directory or file that does not
/// load.
pub fn load_corpus(settings: &Hl7v2Settings) -> Result<Corpus, CorpusError> {
    let mut corpus = Corpus::load(&settings.concept_maps)?.with_shipped_supplements()?;
    for supplement in &settings.supplements {
        corpus = corpus.supplement(supplement)?;
    }
    Ok(corpus)
}

/// The face: what one frame's answer is built from.
#[derive(Debug)]
pub struct Face {
    /// The facade whose ingest service the messages enter.
    facade: Arc<Facade>,
    /// The v2-to-FHIR `ConceptMaps`.
    corpus: Corpus,
    /// The terminology server the table maps are translated on.
    terminology: Option<ferrobridge_term::client::Client>,
    /// The facade's settings, under the face's EHR policy.
    settings: crate::facade::Settings,
    /// The character set a message with an empty MSH-18 is read in.
    charset: Charset,
    /// What an entry no program maps does to the message.
    unmapped: UnmappedEntries,
    /// The profile each resource type claims when the run wrote none.
    profiles: BTreeMap<String, String>,
    /// Whether an `AE` or an `AR` also logs the counted outcomes.
    log_outcomes: bool,
    /// The sending facilities the face accepts; empty accepts any.
    senders: Vec<Sender>,
}

/// One sending facility the face accepts, matched against MSH-4.
///
/// MSH-4 is an `HD`: a namespace id in HD.1, or a universal id in HD.2 with
/// its type in HD.3 (HL7 v2.5.1 chapter 2, `HD` data type). No specification
/// governs an allow list: our own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sender {
    /// A facility named by its namespace id, HD.1.
    Namespace(String),
    /// A facility named by its universal id, HD.2, of the type HD.3.
    Universal {
        /// HD.2.
        id: String,
        /// HD.3, such as `ISO`.
        kind: String,
    },
}

impl Sender {
    /// Returns whether MSH-4 of `message` names this facility.
    #[must_use]
    pub fn names(&self, message: &Message) -> bool {
        let facility = message
            .header()
            .and_then(|header| header.field(4))
            .and_then(|field| field.repetitions().first());
        let component = |position: usize| {
            facility
                .and_then(|facility| facility.component(position))
                .and_then(parse::Component::text)
        };
        match self {
            Self::Namespace(namespace) => component(1) == Some(namespace.as_str()),
            Self::Universal { id, kind } => {
                component(2) == Some(id.as_str()) && component(3) == Some(kind.as_str())
            }
        }
    }
}

/// Returns whether `senders` accepts the sending facility of `message`; an
/// empty list accepts any.
#[must_use]
pub fn admitted(senders: &[Sender], message: &Message) -> bool {
    senders.is_empty() || senders.iter().any(|sender| sender.names(message))
}

impl Face {
    /// Returns the face over `facade`, `corpus` and `terminology` that
    /// `settings` describes.
    #[must_use]
    pub fn new(
        facade: Arc<Facade>,
        corpus: Corpus,
        terminology: Option<ferrobridge_term::client::Client>,
        settings: &Hl7v2Settings,
    ) -> Self {
        let mut ingest = facade.settings().clone();
        if let Some(policy) = settings.ehr_policy {
            ingest.ehr_policy = policy;
        }
        Self {
            facade,
            corpus,
            terminology,
            settings: ingest,
            charset: settings.default_charset,
            unmapped: settings.unmapped,
            profiles: settings.profiles.clone(),
            log_outcomes: settings.log_outcomes,
            senders: settings.senders.clone(),
        }
    }

    /// Answers one message's bytes: the acknowledgment's bytes, or `None`
    /// when no header can be read to address one.
    ///
    /// A message from a sending facility the face does not accept is counted
    /// as refused on `connection` and is never mapped.
    pub async fn answer(&self, message: &[u8], connection: &Connection) -> Option<Vec<u8>> {
        let header = Header::from_bytes(message);
        let span = tracing::info_span!(
            "hl7v2_message",
            control_id = header.as_ref().and_then(Header::control_id),
            message_type = header.as_ref().and_then(Header::type_code),
        );
        async {
            let stamped = Stamped::now();
            let stamp = stamped.stamp();
            match inbound::receive(message, self.charset, structure_for, stamp) {
                Received::Unanswerable => {
                    tracing::warn!("the message names no readable header and is not answered");
                    None
                }
                Received::Answered { code, reply } => {
                    tracing::info!(ack = code.as_str(), "answered before mapping");
                    Some(reply)
                }
                Received::Parsed(inbound) => {
                    if !admitted(&self.senders, inbound.parsed().message()) {
                        connection.refuse();
                        return self.acknowledge(
                            &inbound,
                            &Settled::Sender,
                            &Tally::default(),
                            &stamped,
                        );
                    }
                    self.settle(&inbound, &stamped).await
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Maps and ingests one parsed message and returns its acknowledgment.
    async fn settle(&self, inbound: &Inbound, stamped: &Stamped) -> Option<Vec<u8>> {
        let (settled, tally) = self.ingest(inbound, stamped).await;
        self.acknowledge(inbound, &settled, &tally, stamped)
    }

    /// Returns the acknowledgment `settled` owes, logging its code and counts.
    fn acknowledge(
        &self,
        inbound: &Inbound,
        settled: &Settled,
        tally: &Tally,
        stamped: &Stamped,
    ) -> Option<Vec<u8>> {
        let (code, mut errors) = answer::acknowledgment(settled);
        for error in &mut errors {
            error.text = printable(&error.text);
        }
        tracing::info!(
            ack = code.as_str(),
            committed = tally.committed,
            skipped = tally.skipped,
            errors = errors.len(),
            "message acknowledged"
        );
        if self.log_outcomes && code != Code::Accept {
            for (kind, count) in &tally.outcomes {
                tracing::debug!(outcome = kind, count, "counted outcome of the run");
            }
        }
        match inbound.answer(code, &errors, stamped.stamp()) {
            Ok(reply) => Some(reply),
            Err(error) => {
                tracing::warn!(error = %error, "the acknowledgment cannot be encoded");
                None
            }
        }
    }

    /// Runs one parsed message through the guide and the ingest service.
    async fn ingest(&self, inbound: &Inbound, stamped: &Stamped) -> (Settled, Tally) {
        let mut tally = Tally::default();
        let message = inbound.parsed().message();
        let Some(control) = message
            .control_id()
            .and_then(|control| MessageControlId::new(control).ok())
        else {
            return (Settled::Unkeyed, tally);
        };
        let mapped = match ferrobridge_hl7v2::map::map(
            inbound.parsed(),
            &self.corpus,
            self.terminology.as_ref(),
        )
        .await
        {
            Ok(mapped) => mapped,
            Err(error) => return (Settled::Unmapped(error), tally),
        };
        tally.outcomes = mapped.counts();
        let type_code = [message.message_type(1), message.message_type(2)]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<&str>>()
            .join("^");
        // NOTE: no specification governs this: our own design; MSH-9 is
        // required, so a type the parse let through empty is named by the structure.
        let message_type = MessageType::new(type_code)
            .or_else(|_| MessageType::new(inbound.parsed().structure_name()));
        let Ok(message_type) = message_type else {
            return (Settled::Unkeyed, tally);
        };
        let mut bundle = mapped.into_bundle();
        claim_profiles(&mut bundle, &self.profiles);
        link_subjects(&mut bundle);
        let provenance = Provenance::Item(SourceItem::message(control, message_type));
        let client = match crate::cdr::ids::RequestId::new(&stamped.control_id) {
            Ok(id) => self.facade.client().with_request_id(id),
            Err(_) => self.facade.client().clone(),
        };
        let outcome = self
            .facade
            .ingest_under(client, &self.settings)
            .ingest_bundle(&bundle, self.unmapped, &provenance)
            .await;
        match outcome {
            Ok(ingested) => {
                tally.committed = ingested.committed().count();
                tally.skipped = ingested.skipped().count();
                (Settled::Ingested(Box::new(ingested)), tally)
            }
            Err(refused) => (Settled::Refused(refused), tally),
        }
    }
}

impl Handler for Face {
    async fn handle(&self, message: Vec<u8>, connection: &Connection) -> Option<Vec<u8>> {
        self.answer(&message, connection).await
    }

    fn malformed(&self, kind: Malformed, partial: &[u8]) -> Option<Vec<u8>> {
        let stamped = Stamped::now();
        ack::reject_frame(partial, &kind.to_string(), stamped.stamp())
    }
}

/// The acknowledgment's own MSH-10 and MSH-7.
#[derive(Debug, Clone)]
struct Stamped {
    /// MSH-10: twenty hexadecimal digits of a random UUID.
    control_id: String,
    /// MSH-7: the instant in UTC, as a v2 `DTM` with its offset.
    timestamp: String,
}

impl Stamped {
    /// Returns the stamp of an acknowledgment written now.
    fn now() -> Self {
        let uuid = uuid::Uuid::new_v4().simple().to_string();
        // NOTE: no specification governs this: our own design; twenty characters
        // fit the MSH-10 of every 2.x version the face reads.
        let control_id = uuid.get(..20).map_or_else(|| uuid.clone(), str::to_owned);
        let timestamp = format!("{}+0000", jiff::Timestamp::now().strftime("%Y%m%d%H%M%S"));
        Self {
            control_id,
            timestamp,
        }
    }

    /// Returns the stamp the acknowledgment renders.
    fn stamp(&self) -> Stamp<'_> {
        Stamp {
            control_id: &self.control_id,
            timestamp: &self.timestamp,
        }
    }
}

/// Returns `text` with every character outside printable ASCII replaced by
/// `?`, so an `ERR-8` encodes in every character set a message can declare.
fn printable(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '?'
            }
        })
        .collect()
}

/// Writes the configured profile into `meta.profile` of every entry whose
/// resource type has one and whose resource claims none.
///
/// The facade selects a program by `meta.profile`, and the v2-to-FHIR maps
/// write none, so this is the one place a message's resources are pointed at
/// the context that maps them (no specification governs this: our own
/// design).
pub fn claim_profiles(bundle: &mut Value, profiles: &BTreeMap<String, String>) {
    if profiles.is_empty() {
        return;
    }
    let Value::Object(document) = bundle else {
        return;
    };
    let Some(Value::Array(entries)) = document.get_mut("entry") else {
        return;
    };
    for entry in entries {
        let Value::Object(entry) = entry else {
            continue;
        };
        let Some(Value::Object(resource)) = entry.get_mut("resource") else {
            continue;
        };
        let Some(profile) = resource
            .get("resourceType")
            .and_then(Value::as_str)
            .and_then(|resource_type| profiles.get(resource_type))
        else {
            continue;
        };
        let profile = Value::String(profile.clone());
        let meta = resource
            .entry(String::from("meta"))
            .or_insert_with(|| Value::Object(BTreeMap::new()));
        let Value::Object(meta) = meta else {
            continue;
        };
        let claimed = meta
            .get("profile")
            .and_then(Value::as_array)
            .is_some_and(|claimed| !claimed.is_empty());
        if !claimed {
            meta.insert(String::from("profile"), Value::Array(vec![profile]));
        }
    }
}

/// Points every entry that names no subject at the message's one `Patient`.
///
/// The guide's maps write no `subject`, and its guidelines leave the
/// relationships between the resources one message builds to the
/// implementer ("the PID segment will create a Patient resource which is
/// then ... referenced by the Encounter resource", `mapping_guidelines.md`).
/// So an entry whose R4 type has a `subject` or a `patient` reference, and
/// which carries neither, references the Bundle's `Patient` by its
/// `fullUrl`; the ingest then reads the person from that entry's identifier.
/// A Bundle with no `Patient`, or with more than one, is left as it is and
/// its entries are refused for naming no subject (no specification governs
/// this: our own design).
pub fn link_subjects(bundle: &mut Value) {
    let Value::Object(document) = bundle else {
        return;
    };
    let Some(Value::Array(entries)) = document.get_mut("entry") else {
        return;
    };
    let patients: Vec<String> = entries
        .iter()
        .filter(|entry| {
            entry
                .get("resource")
                .and_then(|resource| resource.get("resourceType"))
                .and_then(Value::as_str)
                == Some("Patient")
        })
        .filter_map(|entry| entry.get("fullUrl").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    let [patient] = patients.as_slice() else {
        return;
    };
    for entry in entries {
        let Value::Object(entry) = entry else {
            continue;
        };
        let Some(Value::Object(resource)) = entry.get_mut("resource") else {
            continue;
        };
        if resource.contains_key("subject") || resource.contains_key("patient") {
            continue;
        }
        let Some(resource_type) = resource.get("resourceType").and_then(Value::as_str) else {
            continue;
        };
        let element = ["subject", "patient"].into_iter().find(|name| {
            fhir_types::r4::schema::SCHEMAS
                .element(&format!("{resource_type}.{name}"))
                .is_some_and(|(_, field)| field.types == ["Reference"] && !field.many)
        });
        if let Some(element) = element {
            let reference =
                BTreeMap::from([(String::from("reference"), Value::String(patient.clone()))]);
            resource.insert(String::from(element), Value::Object(reference));
        }
    }
}

/// Whether the MLLP listener is accepting, for the readiness probe.
#[derive(Debug, Clone, Default)]
pub struct Listening(Arc<AtomicBool>);

impl Listening {
    /// Returns whether the listener is accepting.
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Records whether the listener is accepting.
    pub(crate) fn set(&self, up: bool) {
        self.0.store(up, Ordering::Release);
    }
}

/// How the MLLP listener runs.
#[derive(Debug, Clone, Copy)]
pub struct Listener {
    /// The frame codec, with its ceiling.
    pub codec: Codec,
    /// How long a connection and a frame may wait on the peer.
    pub timeouts: Timeouts,
    /// How long the drain may take after the stop signal.
    pub drain: Duration,
}

impl Listener {
    /// Returns the listener `settings` describe, draining within `drain`.
    #[must_use]
    pub const fn of(settings: &Hl7v2Settings, drain: Duration) -> Self {
        Self {
            codec: Codec::new(settings.frame_limit),
            timeouts: settings.timeouts,
            drain,
        }
    }
}

/// Serves `face` on an already-bound listener until `shutdown` completes,
/// then lets every connection finish the message it is answering within the
/// drain.
///
/// A message in flight when the signal arrives is mapped, committed and
/// acknowledged; a connection still open when the drain elapses is dropped,
/// as the HTTP server's are ([`crate::serve_until`]). `listening` reads up
/// from the first accept until the signal.
///
/// # Errors
///
/// Returns the I/O error of a failed `accept`.
pub async fn serve_until<F>(
    socket: TcpListener,
    face: Arc<Face>,
    listener: Listener,
    listening: Listening,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let signalled = Arc::new(tokio::sync::Notify::new());
    let inner = Arc::clone(&signalled);
    let flag = listening.clone();
    listening.set(true);
    let server = mllp::serve_with(
        socket,
        face,
        listener.codec,
        listener.timeouts,
        async move {
            shutdown.await;
            flag.set(false);
            inner.notify_one();
        },
    );
    let mut server = std::pin::pin!(server);
    let result = tokio::select! {
        result = &mut server => Some(result),
        () = signalled.notified() => None,
    };
    if let Some(result) = result {
        listening.set(false);
        return result;
    }
    if let Ok(result) = tokio::time::timeout(listener.drain, server).await {
        return result;
    }
    tracing::warn!(
        drain_ms = listener.drain.as_millis(),
        "the MLLP drain did not finish in time; the remaining connections are dropped"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Sender, admitted, claim_profiles, link_subjects, printable};
    use fhir_types::codec::Value;
    use std::collections::BTreeMap;

    fn bundle() -> Value {
        Value::from_serde_json(serde_json::json!({
            "resourceType": "Bundle",
            "type": "message",
            "entry": [
                { "fullUrl": "urn:uuid:1", "resource": { "resourceType": "MessageHeader" } },
                { "fullUrl": "urn:uuid:2", "resource": { "resourceType": "Observation" } },
                { "fullUrl": "urn:uuid:3", "resource": {
                    "resourceType": "Observation",
                    "meta": { "profile": ["http://example.org/fhir/StructureDefinition/own"] }
                } }
            ]
        }))
    }

    #[test]
    fn a_configured_profile_is_claimed_only_where_the_run_claimed_none() {
        let mut bundle = bundle();
        let profiles = BTreeMap::from([(
            String::from("Observation"),
            String::from("http://example.org/fhir/StructureDefinition/lab"),
        )]);
        claim_profiles(&mut bundle, &profiles);
        let claimed = |index: usize| {
            bundle
                .get("entry")
                .and_then(Value::as_array)
                .and_then(|entries| entries.get(index))
                .and_then(|entry| entry.get("resource"))
                .and_then(|resource| resource.get("meta"))
                .and_then(|meta| meta.get("profile"))
                .and_then(Value::as_array)
                .and_then(|profiles| profiles.first())
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        assert_eq!(None, claimed(0), "a type with no configured profile");
        assert_eq!(
            Some(String::from(
                "http://example.org/fhir/StructureDefinition/lab"
            )),
            claimed(1)
        );
        assert_eq!(
            Some(String::from(
                "http://example.org/fhir/StructureDefinition/own"
            )),
            claimed(2),
            "a profile the run wrote stands"
        );
    }

    /// Returns the `reference` of `element` on entry `index` of `bundle`.
    fn referenced(bundle: &Value, index: usize, element: &str) -> Option<String> {
        bundle
            .get("entry")
            .and_then(Value::as_array)
            .and_then(|entries| entries.get(index))
            .and_then(|entry| entry.get("resource"))
            .and_then(|resource| resource.get(element))
            .and_then(|reference| reference.get("reference"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    #[test]
    fn an_entry_naming_no_subject_references_the_one_patient() {
        let mut bundle = Value::from_serde_json(serde_json::json!({
            "resourceType": "Bundle",
            "entry": [
                { "fullUrl": "urn:uuid:p", "resource": { "resourceType": "Patient" } },
                { "fullUrl": "urn:uuid:o", "resource": { "resourceType": "Observation" } },
                { "fullUrl": "urn:uuid:i", "resource": { "resourceType": "Immunization" } },
                { "fullUrl": "urn:uuid:h", "resource": { "resourceType": "MessageHeader" } },
                { "fullUrl": "urn:uuid:s", "resource": {
                    "resourceType": "Observation",
                    "subject": { "reference": "Patient/other" }
                } }
            ]
        }));
        link_subjects(&mut bundle);
        assert_eq!(
            Some(String::from("urn:uuid:p")),
            referenced(&bundle, 1, "subject")
        );
        assert_eq!(
            Some(String::from("urn:uuid:p")),
            referenced(&bundle, 2, "patient")
        );
        assert_eq!(
            None,
            referenced(&bundle, 3, "subject"),
            "no subject element"
        );
        assert_eq!(
            Some(String::from("Patient/other")),
            referenced(&bundle, 4, "subject"),
            "a subject the run wrote stands"
        );
    }

    #[test]
    fn two_patients_leave_every_subject_unset() {
        let mut bundle = Value::from_serde_json(serde_json::json!({
            "resourceType": "Bundle",
            "entry": [
                { "fullUrl": "urn:uuid:p", "resource": { "resourceType": "Patient" } },
                { "fullUrl": "urn:uuid:q", "resource": { "resourceType": "Patient" } },
                { "fullUrl": "urn:uuid:o", "resource": { "resourceType": "Observation" } }
            ]
        }));
        link_subjects(&mut bundle);
        assert_eq!(None, referenced(&bundle, 2, "subject"));
    }

    /// Returns the lexed message whose MSH-4 is `facility`.
    fn from(facility: &str) -> ferrobridge_hl7v2::parse::Message {
        let text = format!(
            "MSH|^~\\&|LAB|{facility}|EHR|SOUTH|20260925120000+0200||ORU^R01^ORU_R01|MSG-1|P|2.5.1\r"
        );
        ferrobridge_hl7v2::parse::lex::lex(&text, ferrobridge_hl7v2::decode::Charset::Ascii)
            .expect("the header lexes")
            .message
    }

    #[test]
    fn an_empty_allow_list_admits_any_sender() {
        assert!(admitted(&[], &from("NORTHLAB")));
    }

    #[test]
    fn a_sender_is_admitted_by_its_namespace_id_or_its_universal_id() {
        let senders = [
            Sender::Namespace(String::from("NORTHLAB")),
            Sender::Universal {
                id: String::from("1.2.3.4"),
                kind: String::from("ISO"),
            },
        ];
        assert!(admitted(&senders, &from("NORTHLAB")));
        assert!(admitted(&senders, &from("^1.2.3.4^ISO")));
        assert!(!admitted(&senders, &from("SOUTHLAB")));
        assert!(
            !admitted(&senders, &from("^1.2.3.4^DNS")),
            "the type must match"
        );
        assert!(
            !admitted(&senders, &from("")),
            "an empty MSH-4 names no listed sender"
        );
    }

    #[test]
    fn an_err_text_keeps_printable_ascii_only() {
        assert_eq!("M?ller: no mapping", printable("M\u{fc}ller: no mapping"));
        assert_eq!("a?b", printable("a\rb"));
    }
}
