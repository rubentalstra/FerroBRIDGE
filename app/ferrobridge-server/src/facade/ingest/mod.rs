// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ingest service: one resource or one Bundle, mapped and committed.
//!
//! Every face that writes clinical content into the CDR runs one pipeline.
//! The inbound resource is mapped through its FHIRconnect program with the
//! origin on the engine defaults, the EHR is resolved or created by subject
//! under the configured policy, the built composition is re-read through the
//! strict RM reader, the commit goes out over ITS-REST, and a single write
//! records its identity binding and the consumed source version. [`Ingest`]
//! owns that pipeline over plain values, so the FHIR facade's handlers and a
//! face that is no HTTP interaction share one path (no specification governs
//! this: our own design).
//!
//! A refusal is a [`Refused`]: the status and the `OperationOutcome` issues a
//! handler renders, left unrendered so the service carries no transport.

mod binding;
mod claim;
mod entries;
mod pairing;
mod reconcile;
mod refusal;
mod single;
mod transaction;

use fhir_types::codec::Value;
use fhirconnect::engine::origin::SourceItem;
use fhirconnect::engine::traverse::error::EngineError;
use fhirconnect::resolve::program::binding::TemplateId;
use http::StatusCode;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::CdrClient;
use crate::cdr::Returned;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::EhrId;
use crate::facade::Settings;
use crate::facade::commit;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::PersonId;
use crate::facade::identity::claims::Claims;
use crate::facade::identity::record::CommittedSource;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::store::Store;
use crate::facade::identity::store::StoreError;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::chain;
use crate::facade::programs::Loaded;
use crate::facade::programs::Programs;
use crate::facade::request::Inbound;
use crate::facade::status;

use binding::representation;

/// What a Bundle entry no loaded program maps does to the Bundle.
///
/// No specification governs this: our own design. A FHIR `transaction` is all
/// or nothing (<https://hl7.org/fhir/R4/http.html#transaction>), so the facade
/// refuses; a face whose Bundle carries resources beside the ones it commits
/// skips them and reports how many it skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmappedEntries {
    /// The whole Bundle is refused, naming every such entry.
    Refuse,
    /// The entry is skipped with a typed reason, and the others commit.
    SkipAndCount,
}

/// What each composition's `FEEDER_AUDIT` names as the item it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// Each composition names the resource it was mapped from, by its
    /// `resourceType`, `id` and `meta.versionId`.
    EachResource,
    /// Every composition names this one item, such as the message a face
    /// received.
    Item(SourceItem),
}

impl Provenance {
    /// Returns the source item the composition mapped from `document` names.
    fn source_for(&self, document: &Value) -> Option<SourceItem> {
        match self {
            Self::EachResource => SourceItem::of(document),
            Self::Item(item) => Some(item.clone()),
        }
    }
}

/// A refusal the service answers with, before any transport renders it.
///
/// The status is the one the facade answers on the wire, from the status
/// table where a CDR answer decided it, and the issues are the
/// `OperationOutcome` content in order.
#[derive(Debug, Clone)]
pub struct Refused {
    /// The status the refusal answers with.
    status: StatusCode,
    /// The issues, in the order they are rendered.
    issues: Vec<Issue>,
    /// The `ETag` the refusal carries, when the status table states one.
    entity_tag: Option<String>,
    /// The `WWW-Authenticate` challenge the CDR sent, when it sent one.
    challenge: Option<String>,
}

impl Refused {
    /// Returns the refusal carrying one issue under `status`.
    #[must_use]
    pub fn new(status: StatusCode, issue: Issue) -> Self {
        Self::of(status, vec![issue])
    }

    /// Returns the refusal carrying `issues` under `status`.
    #[must_use]
    pub const fn of(status: StatusCode, issues: Vec<Issue>) -> Self {
        Self {
            status,
            issues,
            entity_tag: None,
            challenge: None,
        }
    }

    /// Returns the refusal one status-table answer decides.
    #[must_use]
    pub fn of_answer(answer: &status::Answer) -> Self {
        Self {
            status: answer.status(),
            issues: vec![answer.issue().clone()],
            entity_tag: answer.entity_tag().map(str::to_owned),
            challenge: answer.challenge().map(str::to_owned),
        }
    }

    /// Returns the status the refusal answers with.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the issues, in order.
    #[must_use]
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// Returns the `ETag` the refusal carries.
    #[must_use]
    pub fn entity_tag(&self) -> Option<&str> {
        self.entity_tag.as_deref()
    }

    /// Returns the `WWW-Authenticate` challenge the refusal propagates.
    #[must_use]
    pub fn challenge(&self) -> Option<&str> {
        self.challenge.as_deref()
    }
}

/// What one committed single write produced.
#[derive(Debug, Clone)]
pub struct Written {
    /// The logical id the resource now has.
    pub id: FhirResourceId,
    /// The EHR the composition lives in.
    pub ehr_id: EhrId,
    /// The composition version the commit produced.
    pub version: ObjectVersionId,
    /// The composition as it now stands.
    pub composition: CanonicalComposition,
    /// Whether this write committed the composition or found it committed.
    pub delivery: Delivery,
}

/// How the composition one single write answers with reached the CDR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// This write committed the composition.
    Committed,
    /// An earlier transaction committed the composition and did not finish
    /// binding it, so this write bound it from the contribution read back
    /// and committed nothing.
    Reconciled,
    /// An earlier delivery of the same source produced the composition, so
    /// this write committed nothing and answers where that composition
    /// stands.
    Replayed,
}

/// What the identity map knows of the source one single create carries.
#[derive(Debug)]
pub enum Recognised {
    /// The map consumed no version of the source's `id`: the create runs
    /// [`Ingest::ingest_resource`], which also finishes the binding a
    /// transaction left undone.
    Unknown,
    /// The source repeats one the map consumed, so nothing is committed and
    /// the answer names the composition the first delivery produced.
    Replayed(Written),
    /// The map consumed another version of the source's `id`, so the create
    /// commits a later version of the composition that version produced.
    Revised(ConsumedSource),
}

/// Why an entry of a Bundle was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// No loaded program maps the entry's resource type.
    NoProgramForType,
    /// Programs map the type, and none claims a profile the entry claims.
    NoProgramForProfiles {
        /// The profiles the entry claims, in document order.
        profiles: Vec<String>,
    },
}

/// One Bundle entry the service skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The `fullUrl` of the entry, or its position when it has none.
    pub full_url: String,
    /// The `resourceType` the entry declares.
    pub resource_type: String,
    /// Why no program ran over it.
    pub reason: SkipReason,
}

/// One Bundle entry the service committed.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    /// The `fullUrl` of the entry, or its position when it has none.
    pub full_url: String,
    /// The `resourceType` the entry declares.
    pub resource_type: String,
    /// The template of the composition the entry produced.
    pub template_id: String,
    /// The logical id the entry's resource has.
    pub id: FhirResourceId,
    /// The composition version the entry stands at.
    pub version: ObjectVersionId,
    /// The change this delivery committed for the entry: a first version of
    /// a composition, or a later version of the one an earlier version of
    /// the entry's `id` produced. Nothing when this delivery committed
    /// nothing for it.
    pub change: Option<commit::Change>,
}

/// How the committed entries of a Bundle reached the CDR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Commit {
    /// Every committed entry went in as one version of this CONTRIBUTION.
    Contribution(ContributionUid),
    /// The identity map already recorded every entry's source as consumed, or
    /// as committed by one contribution whose binding it then finished, so
    /// nothing was committed and each entry names the composition its first
    /// delivery produced.
    AlreadyConsumed,
}

/// What happened to one Bundle entry.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryOutcome {
    /// The entry mapped and its composition is stored.
    Committed(Committed),
    /// No program ran over the entry.
    Skipped(Skipped),
}

/// What one ingested Bundle produced.
///
/// Every committed entry went into the CDR as one version of one
/// CONTRIBUTION (`ehr-codegen.openapi.yaml`, `contribution_create`), or, for a
/// Bundle whose every source the identity map already recorded, stands where
/// its first delivery put it.
#[derive(Debug, Clone)]
pub struct Ingested {
    /// The EHR every composition lives in.
    ehr_id: EhrId,
    /// How the committed entries reached the CDR.
    commit: Commit,
    /// One outcome per entry, in Bundle order.
    entries: Vec<EntryOutcome>,
}

impl Ingested {
    /// Returns the EHR every composition lives in.
    #[must_use]
    pub const fn ehr_id(&self) -> &EhrId {
        &self.ehr_id
    }

    /// Returns how the committed entries reached the CDR.
    #[must_use]
    pub const fn commit(&self) -> &Commit {
        &self.commit
    }

    /// Returns the contribution that carries every committed composition,
    /// when this delivery committed one.
    #[must_use]
    pub const fn contribution(&self) -> Option<&ContributionUid> {
        match self.commit {
            Commit::Contribution(ref uid) => Some(uid),
            Commit::AlreadyConsumed => None,
        }
    }

    /// Returns one outcome per entry, in Bundle order.
    #[must_use]
    pub fn entries(&self) -> &[EntryOutcome] {
        &self.entries
    }

    /// Returns the committed entries, in Bundle order.
    pub fn committed(&self) -> impl Iterator<Item = &Committed> {
        self.entries.iter().filter_map(|entry| match entry {
            EntryOutcome::Committed(committed) => Some(committed),
            EntryOutcome::Skipped(_) => None,
        })
    }

    /// Returns the skipped entries, in Bundle order.
    pub fn skipped(&self) -> impl Iterator<Item = &Skipped> {
        self.entries.iter().filter_map(|entry| match entry {
            EntryOutcome::Skipped(skipped) => Some(skipped),
            EntryOutcome::Committed(_) => None,
        })
    }
}

/// The ingest pipeline over one deployment's programs, store, CDR and
/// settings.
#[derive(Debug)]
pub struct Ingest<'a> {
    /// The compiled programs.
    programs: &'a Programs,
    /// The identity map.
    store: &'a dyn Store,
    /// The source keys the in-flight deliveries into that map hold.
    claims: &'a Claims,
    /// The CDR client every call of this ingest goes through.
    client: CdrClient,
    /// What the deployment configured.
    settings: &'a Settings,
}

/// One Bundle entry that mapped, ready to commit.
#[derive(Debug)]
struct Mapped<'p> {
    /// The `fullUrl` the Bundle gave the entry, for a refusal that names it.
    full_url: String,
    /// The entry's position in the Bundle.
    position: usize,
    /// The resource the entry carries.
    inbound: Inbound,
    /// The program the entry ran.
    program: &'p Loaded,
    /// The template of the composition the entry produced.
    template_id: String,
    /// The composition as the engine built it.
    built: CanonicalComposition,
    /// The composition as the strict reader read it back.
    composition: Box<Composition>,
    /// The instant the engine defaulted every clock-derived time from.
    instant: String,
}

/// Which versions of a committed contribution the entries being bound
/// account for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Every version is the composition of one of the entries.
    Whole,
    /// The entries are some of the versions, as one resource of a
    /// transaction is; a version no entry matches is passed over.
    Part,
}

/// What mapping one Bundle entry produced.
#[derive(Debug)]
enum Mapping<'p> {
    /// The entry mapped, for the subject it names.
    Mapped(Box<Mapped<'p>>, PersonId),
    /// No program ran: the skip, and the issue that refuses the Bundle.
    Unmapped(Skipped, Issue),
}

/// Where one entry of a Bundle stands before the commit.
#[derive(Debug)]
enum Pending {
    /// The entry mapped: its index among the mapped entries.
    Mapped(usize),
    /// No program ran over the entry.
    Skipped(Skipped),
}

/// Every entry of a Bundle, mapped and partitioned.
#[derive(Debug, Default)]
struct Partition<'p> {
    /// The entries that mapped, in Bundle order.
    mapped: Vec<Mapped<'p>>,
    /// One slot per entry that mapped or was skipped, in Bundle order.
    pending: Vec<Pending>,
    /// The issues that refuse the Bundle.
    failures: Vec<Issue>,
    /// The one subject every mapped entry names.
    subject: Option<PersonId>,
}

/// Where one committed entry stands.
#[derive(Debug)]
struct Placed {
    /// The logical id the entry's resource has.
    id: FhirResourceId,
    /// The composition version the entry stands at.
    version: ObjectVersionId,
    /// The change this delivery committed for the entry, when it committed
    /// one.
    change: Option<commit::Change>,
}

/// What the identity map records of one Bundle entry's source.
#[derive(Debug, Default)]
struct Recorded {
    /// What consumed the exact source.
    consumed: Option<ConsumedSource>,
    /// The contribution that committed the exact source.
    committed: Option<CommittedSource>,
    /// What consumed another version of the source's `id`, whose composition
    /// the entry revises.
    revised: Option<ConsumedSource>,
}

/// Where the composition a consumed source produced stands now.
#[derive(Debug)]
struct Standing {
    /// The EHR the composition lives in.
    ehr_id: EhrId,
    /// The logical id the resource has.
    id: FhirResourceId,
    /// The version the composition stands at.
    version: ObjectVersionId,
    /// The composition the CDR holds at that version.
    composition: Composition,
}

impl Standing {
    /// Returns the answer of a write that found this composition and
    /// committed nothing, with `built` as the mapping the answer renders
    /// through.
    fn written(self, built: &CanonicalComposition) -> Result<Written, Refused> {
        let composition =
            representation(Returned::Representation(Box::new(self.composition)), built)?;
        Ok(Written {
            id: self.id,
            ehr_id: self.ehr_id,
            version: self.version,
            composition,
            delivery: Delivery::Replayed,
        })
    }
}

impl<'a> Ingest<'a> {
    /// Returns an ingest over `programs`, `store`, `client` and `settings`.
    ///
    /// `claims` is the one set of in-flight source keys every ingest into
    /// `store` shares, so two deliveries of one source cannot both commit.
    #[must_use]
    pub const fn new(
        programs: &'a Programs,
        store: &'a dyn Store,
        claims: &'a Claims,
        client: CdrClient,
        settings: &'a Settings,
    ) -> Self {
        Self {
            programs,
            store,
            claims,
            client,
            settings,
        }
    }

    /// Returns the CDR client this ingest calls through.
    #[must_use]
    pub const fn client(&self) -> &CdrClient {
        &self.client
    }
}

/// Selects the program one inbound resource runs, pinned by `pin`.
///
/// # Errors
///
/// Returns a `422` [`Refused`] located at `Resource.meta.profile` when no
/// program, or more than one, answers; an ambiguous choice lists the
/// candidates the `templateId` pin can name.
pub fn select<'p>(
    programs: &'p Programs,
    inbound: &Inbound,
    pin: Option<&TemplateId>,
) -> Result<&'p Loaded, Refused> {
    programs.select(inbound.profiles(), pin).map_err(|error| {
        let candidates = programs.candidates(inbound.profiles());
        let issue = if candidates.len() > 1 {
            Issue::error(IssueType::NotSupported).diagnosing(format!(
                "{}; pin the choice with the templateId parameter, whose candidates are: {}",
                error,
                candidates.join(", ")
            ))
        } else {
            Issue::error(IssueType::NotSupported).diagnosing(error.to_string())
        };
        Refused::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            issue.at("Resource.meta.profile"),
        )
    })
}

/// Why the composition the CDR returned cannot stand as the stored one.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RepresentationError {
    /// The returned composition does not read back as canonical JSON.
    #[error("the composition the CDR returned does not read back as canonical JSON")]
    Unreadable {
        /// What the JSON reader reported.
        #[source]
        source: serde_json::Error,
    },
}

/// Returns the refusal one engine error renders as.
///
/// An element the program cannot map refuses the unit (no specification
/// governs this: our own design), so the answer is a `422` whose diagnostics
/// name the mapping and the element the engine refused at.
#[must_use]
pub fn engine_refusal(error: &EngineError) -> Refused {
    Refused::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        Issue::error(IssueType::Processing)
            .diagnosing(chain(error))
            .detailing("the mapping refused this resource"),
    )
}

/// Returns the refusal an identity-store failure renders as.
#[must_use]
pub fn store_refusal(error: &StoreError) -> Refused {
    Refused::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(chain(&error)),
    )
}

#[cfg(test)]
mod tests {
    use super::{Provenance, Refused};
    use crate::facade::outcome::{Issue, IssueType};
    use crate::facade::status;
    use fhir_types::codec::Value;
    use fhirconnect::engine::origin::{MessageControlId, MessageType, SourceItem};
    use http::StatusCode;

    #[test]
    fn each_resource_names_the_resource_it_maps() {
        let document = Value::from_serde_json(serde_json::json!({
            "resourceType": "Observation",
            "id": "obs-1"
        }));
        let source = Provenance::EachResource
            .source_for(&document)
            .expect("a resource names its type");
        assert_eq!("Observation", source.resource_type());
        assert_eq!(Some("obs-1"), source.id());
    }

    #[test]
    fn a_given_item_is_named_whatever_the_resource() {
        let document = Value::from_serde_json(serde_json::json!({
            "resourceType": "Observation",
            "id": "obs-1"
        }));
        let item = SourceItem::message(
            MessageControlId::new("MSG-0001").expect("a non-empty control id"),
            MessageType::new("ORU^R01").expect("a non-empty message type"),
        );
        assert_eq!(
            Some(item.clone()),
            Provenance::Item(item).source_for(&document)
        );
    }

    #[test]
    fn a_status_table_answer_keeps_its_entity_tag_and_its_issue() {
        let answer = status::Answer::new(status::PRECONDITION_FAILED, String::from("stale"))
            .with_entity_tag("2");
        let refused = Refused::of_answer(&answer);
        assert_eq!(StatusCode::PRECONDITION_FAILED, refused.status());
        assert_eq!(Some("2"), refused.entity_tag());
        assert_eq!(None, refused.challenge());
        assert_eq!(1, refused.issues().len());
    }

    #[test]
    fn a_refusal_keeps_its_issues_in_order() {
        let refused = Refused::of(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![
                Issue::error(IssueType::NotSupported),
                Issue::error(IssueType::Required),
            ],
        );
        let codes: Vec<IssueType> = refused.issues().iter().map(Issue::code).collect();
        assert_eq!(vec![IssueType::NotSupported, IssueType::Required], codes);
    }
}
