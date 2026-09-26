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

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use fhir_types::codec::Value;
use fhirconnect::engine::origin::SourceItem;
use fhirconnect::resolve::program::TemplateId;
use fhirconnect::resolve::select::SelectError;
use http::StatusCode;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::generated::ehr::Versionable;
use openehr_its::rest::generated::ehr::client::CompositionCreateOutcome;
use openehr_its::rest::generated::ehr::client::CompositionGetOutcome;
use openehr_its::rest::generated::ehr::client::CompositionUpdateOutcome;
use openehr_its::rest::generated::ehr::client::ContributionCreateOutcome;
use openehr_its::rest::generated::ehr::client::ContributionGetOutcome;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::common::change_control::contribution::Contribution;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::CdrClient;
use crate::cdr::Prefer;
use crate::cdr::Returned;
use crate::cdr::error::CdrError;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::EhrId;
use crate::cdr::ids::versioned_object_uid;
use crate::facade::Settings;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::engine;
use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::IdError;
use crate::facade::identity::PersonId;
use crate::facade::identity::claims::Claim;
use crate::facade::identity::claims::Claims;
use crate::facade::identity::claims::Contended;
use crate::facade::identity::derive;
use crate::facade::identity::derive::EntryKey;
use crate::facade::identity::record::CommittedSource;
use crate::facade::identity::record::CompositionBinding;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::record::Identifier;
use crate::facade::identity::record::SourceVersion;
use crate::facade::identity::store::Store;
use crate::facade::identity::store::StoreError;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::chain;
use crate::facade::programs::Loaded;
use crate::facade::programs::Programs;
use crate::facade::request::Inbound;
use crate::facade::status;

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

    /// Claims the source keys of `inbound` for one delivery.
    ///
    /// The caller takes the claim before it reads the identity map and holds
    /// it until the delivery settles. The claim holds the source's
    /// `resourceType`, `id` and `meta.versionId` ([`SourceVersion::storage_key`])
    /// and its `resourceType` and `id` alone ([`SourceVersion::resource_key`]),
    /// so a second single delivery of the same `id` that arrives meanwhile,
    /// whatever its `meta.versionId`, commits nothing. A resource with no `id`
    /// has no key, and its claim holds nothing. No specification governs the
    /// redelivery rule: our own design.
    ///
    /// # Errors
    ///
    /// Returns a `409` [`Refused`] with a `duplicate` issue when another
    /// in-flight delivery holds a key; the caller retries once that delivery
    /// settles.
    pub fn claim_resource(&self, inbound: &Inbound) -> Result<Claim<'a>, Refused> {
        let source = resource_source(inbound);
        self.claims
            .claim(
                source
                    .iter()
                    .flat_map(|source| [source.storage_key(), source.resource_key()]),
            )
            .map_err(|_contended| {
                let at = source.map_or_else(
                    || String::from(inbound.resource_type()),
                    |source| format!("{}/{}", source.resource_type(), source.id()),
                );
                in_flight(std::iter::once(at))
            })
    }

    /// Returns what the identity map knows of the source a single create of
    /// `inbound` carries.
    ///
    /// The source key is the `resourceType`, the `id` and the
    /// `meta.versionId` ([`SourceVersion`]). No specification governs the
    /// rule that follows: our own design, since R4 `create` says nothing of a
    /// repeated create with a client-assigned `id`
    /// (<https://hl7.org/fhir/R4/http.html#create>) and a `meta.versionId`
    /// changes each time the sender's resource changes
    /// (<https://hl7.org/fhir/R4/resource.html#Meta>).
    ///
    /// - A source whose `id` and `meta.versionId` the map consumed is
    ///   [`Recognised::Replayed`]: nothing is committed.
    /// - A source whose `id` the map consumed at another `meta.versionId` is
    ///   [`Recognised::Revised`].
    /// - A source with no `meta.versionId` whose `id` the map consumed at any
    ///   version is compared with the composition as it stands, masking what
    ///   two mappings of one resource differ in (the `uid` the CDR assigns and
    ///   every time the run filled from its clock, as `pair` masks them).
    ///   The same content is [`Recognised::Replayed`], other content
    ///   [`Recognised::Revised`].
    /// - A source whose key a transaction committed and left unbound, or whose
    ///   `id` the map never consumed, is [`Recognised::Unknown`].
    ///
    /// `claim` is the one [`Ingest::claim_resource`] took for `inbound`.
    ///
    /// # Errors
    ///
    /// Returns a `409` [`Refused`] when the map binds the `id` to more than
    /// one composition, a `422` for a resource the program cannot map or a
    /// composition the strict reader refuses, every CDR refusal of the
    /// composition read through the status table, a `500` when `claim` does
    /// not hold the resource's source key, and a `500` when the identity
    /// store cannot be read or holds an identifier this version cannot read.
    pub async fn recognise(
        &self,
        claim: &Claim<'_>,
        inbound: &Inbound,
        program: &Loaded,
        provenance: &Provenance,
    ) -> Result<Recognised, Refused> {
        let Some(source) = resource_source(inbound) else {
            return Ok(Recognised::Unknown);
        };
        if !claim.holds(&source.storage_key()) {
            return Err(unclaimed());
        }
        let exact = self
            .store
            .consumed(&source)
            .map_err(|error| store_refusal(&error))?;
        let known = if let Some(known) = exact {
            if source.version_id().is_some() {
                let built = self.build(program, inbound, provenance)?;
                let standing = self.standing(&known).await?;
                return Ok(Recognised::Replayed(standing.written(&built)?));
            }
            known
        } else {
            if self
                .store
                .committed(&source)
                .map_err(|error| store_refusal(&error))?
                .is_some()
            {
                return Ok(Recognised::Unknown);
            }
            let versions = self
                .store
                .consumed_versions(&source)
                .map_err(|error| store_refusal(&error))?;
            let Some(known) = one_composition(&source, versions)? else {
                return Ok(Recognised::Unknown);
            };
            if source.version_id().is_some() {
                return Ok(Recognised::Revised(known));
            }
            known
        };
        let (built, instant) = self
            .run(program, inbound, provenance)
            .map_err(|error| engine_refusal(&error))?;
        let sent = strict_read(&built)?;
        let standing = self.standing(&known).await?;
        if same_content(&sent, &instant, &standing.composition) {
            return Ok(Recognised::Replayed(standing.written(&built)?));
        }
        Ok(Recognised::Revised(known))
    }

    /// Commits the first version of a composition for `inbound`.
    ///
    /// The EHR is the subject's, resolved or created under the configured
    /// policy; the identity binding and the consumed source version are
    /// recorded once the CDR stored the composition.
    ///
    /// A source the identity map records as committed by a transaction and
    /// not consumed is bound as a re-sent transaction binds it: from the
    /// CONTRIBUTION read back, with the version matched to the resource by
    /// its `FEEDER_AUDIT`, committing nothing, and answering
    /// [`Delivery::Reconciled`] with the composition the first delivery
    /// produced. No specification governs the redelivery rule: our own
    /// design.
    ///
    /// `claim` is the one [`Ingest::claim_resource`] took for `inbound`,
    /// held by the caller from before its identity lookup until the answer,
    /// so a concurrent delivery of the same source cannot commit beside this
    /// one.
    ///
    /// # Errors
    ///
    /// Returns a [`Refused`] for a resource that names no usable subject, an
    /// EHR that cannot be resolved, a resource the program cannot map, a
    /// composition the strict reader refuses, every CDR refusal through the
    /// status table, a `500` naming the recorded contribution when it cannot
    /// be read back or none of its versions matches the resource, a `500`
    /// when `claim` does not hold the resource's source key, and an identity
    /// store that cannot be read or written.
    pub async fn ingest_resource(
        &self,
        claim: &Claim<'_>,
        inbound: &Inbound,
        program: &Loaded,
        provenance: &Provenance,
    ) -> Result<Written, Refused> {
        if let Some(source) = resource_source(inbound)
            && !claim.holds(&source.storage_key())
        {
            return Err(unclaimed());
        }
        let subject = inbound
            .subject(&self.settings.subject_namespace)
            .map_err(|error| {
                Refused::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Issue::error(IssueType::Required)
                        .diagnosing(chain(&error))
                        .at(format!("{}.subject", inbound.resource_type())),
                )
            })?;
        if let Some(written) = self
            .reconcile_resource(inbound, program, provenance)
            .await?
        {
            return Ok(written);
        }
        let ehr_id = ehr::resolve(&self.client, self.store, &subject, self.settings.ehr_policy)
            .await
            .map_err(|error| ehr_refusal(&error))?;
        let composition = self.build(program, inbound, provenance)?;
        let rm = strict_read(&composition)?;
        let context = self.commit_context(commit::Change::Creation, &composition)?;
        let answered = self
            .client
            .create_composition(&ehr_id, &rm, &context, Prefer::Representation)
            .await
            .map_err(|error| cdr_refusal(&error))?;
        let (version, stored) = match answered.outcome {
            CompositionCreateOutcome::Created { body, headers } => {
                let version = crate::cdr::version_from_etag(
                    "composition_create",
                    StatusCode::CREATED,
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                let returned = crate::cdr::returned::<Composition>(
                    "composition_create",
                    body.as_ref(),
                    Prefer::Representation,
                )
                .map_err(|error| cdr_refusal(&error))?;
                (version, representation(returned, &composition)?)
            }
            CompositionCreateOutcome::NoContent { headers } => {
                let version = crate::cdr::version_from_etag(
                    "composition_create",
                    StatusCode::NO_CONTENT,
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                (version, composition.clone())
            }
            CompositionCreateOutcome::UnprocessableEntity { body } => {
                return Err(upstream_refusal(
                    status::UNPROCESSABLE,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &body,
                ));
            }
            CompositionCreateOutcome::NotFound { body } => {
                return Err(upstream_refusal(
                    status::NOT_FOUND,
                    StatusCode::NOT_FOUND,
                    &body,
                ));
            }
            CompositionCreateOutcome::BadRequest { body } => {
                return Err(upstream_refusal(
                    status::BAD_REQUEST,
                    StatusCode::BAD_REQUEST,
                    &body,
                ));
            }
        };
        self.record(program, inbound, &ehr_id, &version, &stored)
    }

    /// Binds `inbound` from the contribution a transaction recorded as
    /// committing it, when the identity map holds that record and no
    /// consumed source.
    ///
    /// The contribution is read back and the version whose composition the
    /// resource maps to is found the way a re-sent transaction finds it
    /// ([`pair`]), so the single and the transaction paths share one rule.
    async fn reconcile_resource(
        &self,
        inbound: &Inbound,
        program: &Loaded,
        provenance: &Provenance,
    ) -> Result<Option<Written>, Refused> {
        let Some(source) = resource_source(inbound) else {
            return Ok(None);
        };
        if self
            .store
            .consumed(&source)
            .map_err(|error| store_refusal(&error))?
            .is_some()
        {
            return Ok(None);
        }
        let Some(committed) = self
            .store
            .committed(&source)
            .map_err(|error| store_refusal(&error))?
        else {
            return Ok(None);
        };
        let (ehr_id, contribution) = committed_ids(&committed)?;
        let (built, instant) = self
            .run(program, inbound, provenance)
            .map_err(|error| engine_refusal(&error))?;
        let rm = strict_read(&built)?;
        let mapped = [Mapped {
            full_url: format!("{}/{}", source.resource_type(), source.id()),
            position: 0,
            inbound: inbound.clone(),
            program,
            template_id: String::from(built.template_id()),
            built,
            composition: Box::new(rm),
            instant,
        }];
        let found = self
            .committed_versions(
                &ehr_id,
                &contribution,
                Returned::Minimal,
                &mapped,
                Scope::Part,
            )
            .await?;
        let Ok([(version, stored)]) = <[(ObjectVersionId, Composition); 1]>::try_from(found) else {
            return Err(unbound(
                &contribution,
                "its versions do not bind the resource once",
            ));
        };
        let placed = self.place(&mapped, &[Some(source)], &ehr_id, vec![version.clone()])?;
        let [ref entry] = mapped;
        let Ok([place]) = <[Placed; 1]>::try_from(placed) else {
            return Err(unbound(
                &contribution,
                "the resource could not be bound once",
            ));
        };
        let composition = representation(Returned::Representation(Box::new(stored)), &entry.built)?;
        Ok(Some(Written {
            id: place.id,
            ehr_id,
            version,
            composition,
            delivery: Delivery::Reconciled,
        }))
    }

    /// Commits a later version of the composition `container` holds.
    ///
    /// `preceding` is the version the write follows, so a concurrent writer
    /// meets the `412` the CDR answers.
    ///
    /// # Errors
    ///
    /// Returns a [`Refused`] for a resource the program cannot map, a
    /// composition the strict reader refuses, every CDR refusal through the
    /// status table (a `412` carrying the latest version as its `ETag`), and
    /// an identity store that cannot be written.
    pub async fn revise_resource(
        &self,
        inbound: &Inbound,
        program: &Loaded,
        ehr_id: &EhrId,
        container: &HierObjectId,
        preceding: &ObjectVersionId,
        provenance: &Provenance,
    ) -> Result<Written, Refused> {
        let composition = self.build(program, inbound, provenance)?;
        let rm = strict_read(&composition)?;
        let context = self.commit_context(commit::Change::Modification, &composition)?;
        let answered = self
            .client
            .update_composition(
                ehr_id,
                container,
                preceding,
                &rm,
                &context,
                Prefer::Representation,
            )
            .await
            .map_err(|error| cdr_refusal(&error))?;
        let (version, stored) = match answered.outcome {
            CompositionUpdateOutcome::Ok { body, headers } => {
                let version = crate::cdr::version_from_etag(
                    "composition_update",
                    StatusCode::OK,
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                let returned = crate::cdr::returned::<Composition>(
                    "composition_update",
                    Some(&body),
                    Prefer::Representation,
                )
                .map_err(|error| cdr_refusal(&error))?;
                (version, representation(returned, &composition)?)
            }
            CompositionUpdateOutcome::NoContent { headers } => {
                let version = crate::cdr::version_from_etag(
                    "composition_update",
                    StatusCode::NO_CONTENT,
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                (version, composition.clone())
            }
            CompositionUpdateOutcome::UnprocessableEntity { body } => {
                return Err(upstream_refusal(
                    status::UNPROCESSABLE,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &body,
                ));
            }
            CompositionUpdateOutcome::PreconditionFailed { body, headers } => {
                let latest = crate::cdr::optional_version_from_etag(
                    "composition_update",
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                let mut answer = status::Answer::new(
                    status::PRECONDITION_FAILED,
                    status::diagnostics(StatusCode::PRECONDITION_FAILED, &body),
                );
                if let Some(latest) = latest {
                    answer = answer.with_entity_tag(String::from(latest.version_tree_id().value()));
                }
                return Err(Refused::of_answer(&answer));
            }
            CompositionUpdateOutcome::NotFound { body } => {
                return Err(upstream_refusal(
                    status::NOT_FOUND,
                    StatusCode::NOT_FOUND,
                    &body,
                ));
            }
            CompositionUpdateOutcome::BadRequest { body } => {
                return Err(upstream_refusal(
                    status::BAD_REQUEST,
                    StatusCode::BAD_REQUEST,
                    &body,
                ));
            }
        };
        self.record(program, inbound, ehr_id, &version, &stored)
    }

    /// Maps every entry of `bundle` and commits the mapped ones as one
    /// CONTRIBUTION.
    ///
    /// Every entry is mapped before anything reaches the CDR, so a Bundle
    /// that cannot be mapped in full under `unmapped` commits nothing, and the
    /// refusal names every failing entry by its `fullUrl`. One Bundle writes
    /// into one EHR, so the mapped entries must name one subject. The Bundle's
    /// `type` is the caller's to check.
    ///
    /// Each committed entry records its identity binding and, when it has a
    /// key ([`SourceVersion`]), its consumed source, as a single create does.
    /// A Bundle whose every mapped entry has a key the identity map already
    /// recorded commits nothing and answers [`Commit::AlreadyConsumed`], with
    /// each entry at the composition its first delivery produced; a Bundle
    /// only some of whose entries were consumed is refused, because a
    /// transaction is all or nothing (<https://hl7.org/fhir/R4/http.html#transaction>).
    ///
    /// An entry mapped from a resource whose `id` the map consumed at another
    /// `meta.versionId`, and whose exact key it holds no record of, revises
    /// the composition that version produced, as a single create of it does
    /// ([`Ingest::recognise`]): it goes into the CONTRIBUTION as a
    /// modification following the composition's latest version, beside the
    /// creations of the other entries, and binds to the resource id it
    /// already has. No specification governs the redelivery rule: our own
    /// design.
    ///
    /// The commit is two steps. The contribution uid the CDR answers is
    /// recorded against every keyed entry ([`CommittedSource`]) before any
    /// entry is bound; the versions then come from the answer's CONTRIBUTION
    /// or, when it carries none, from the CONTRIBUTION read back by that uid.
    /// A Bundle whose every entry one recorded contribution committed is bound
    /// from that contribution read back, commits nothing and answers
    /// [`Commit::AlreadyConsumed`], so a retry after a failed binding is safe.
    ///
    /// Every keyed entry is claimed ([`Claims`]) before the identity map is
    /// read and released when the Bundle settles, so of two deliveries of one
    /// Bundle that overlap, the second commits nothing and is refused with a
    /// `409`; sent again after the first settled, it answers as any re-sent
    /// Bundle. No specification governs the redelivery rule: our own design.
    ///
    /// # Errors
    ///
    /// Returns a `422` [`Refused`] naming every entry that does not map (and,
    /// under [`UnmappedEntries::Refuse`], every entry no program maps), a
    /// second subject, two entries with one source key, a Bundle with nothing
    /// to commit, and an EHR that cannot be resolved; a `409` naming every
    /// entry another in-flight delivery holds; a `409` naming every
    /// entry already consumed or committed when not all were; a `409` for an
    /// entry the map binds to more than one composition or to one in another
    /// EHR, and a `422` for two entries that revise one composition; the
    /// CDR's refusal of a revised composition's read; the CDR's
    /// refusal of the contribution through the status table, naming every
    /// mapped entry; a `500` naming the contribution when it cannot be read
    /// back or its versions cannot bind the entries; and a `500` when the
    /// identity store cannot be read or written.
    pub async fn ingest_bundle(
        &self,
        bundle: &Value,
        unmapped: UnmappedEntries,
        provenance: &Provenance,
    ) -> Result<Ingested, Refused> {
        let entries = bundle
            .get("entry")
            .and_then(Value::as_array)
            .unwrap_or_default();
        let partition = self.map_entries(entries, unmapped, provenance);
        if !partition.failures.is_empty() {
            return Err(Refused::of(
                StatusCode::UNPROCESSABLE_ENTITY,
                partition.failures,
            ));
        }
        let Some(ref subject) = partition.subject else {
            return Err(nothing_mapped(&partition));
        };
        let sources = sources_of(&partition.mapped, provenance)?;
        distinct(&partition.mapped, &sources)?;
        let _claim = self
            .claims
            .claim(claim_keys(&sources, provenance))
            .map_err(|contended| contended_entries(&partition.mapped, &sources, &contended))?;
        let mut known = Vec::with_capacity(sources.len());
        let mut committed = Vec::with_capacity(sources.len());
        let mut revised = Vec::with_capacity(sources.len());
        for source in &sources {
            let recorded = match source {
                Some(source) => self.recorded(source, provenance)?,
                None => Recorded::default(),
            };
            known.push(recorded.consumed);
            committed.push(recorded.committed);
            revised.push(recorded.revised);
        }
        let consumed = known.iter().flatten().count();
        if consumed > 0 && consumed == known.len() {
            return self.replay(partition, &known).await;
        }
        if let Some(first) = one_contribution(&committed) {
            return self.reconcile(partition, &sources, first).await;
        }
        let seen: Vec<bool> = known
            .iter()
            .zip(&committed)
            .map(|(recorded, contribution)| recorded.is_some() || contribution.is_some())
            .collect();
        if seen.contains(&true) {
            return Err(partly_consumed(&partition.mapped, &seen));
        }
        let ehr_id = ehr::resolve(&self.client, self.store, subject, self.settings.ehr_policy)
            .await
            .map_err(|error| ehr_refusal(&error))?;
        let preceding = self.preceding(&ehr_id, &partition.mapped, &revised).await?;
        let (contribution, returned) = self
            .commit_all(&ehr_id, &partition.mapped, &preceding)
            .await?;
        self.record_commitment(&sources, &ehr_id, &contribution)?;
        let versions = self
            .committed_versions(
                &ehr_id,
                &contribution,
                returned,
                &partition.mapped,
                Scope::Whole,
            )
            .await?;
        let mut placed = self.place(&partition.mapped, &sources, &ehr_id, versions_of(versions))?;
        for (place, prior) in placed.iter_mut().zip(&preceding) {
            place.change = Some(change_of(prior.as_ref()));
        }
        Ok(Ingested {
            ehr_id,
            commit: Commit::Contribution(contribution),
            entries: outcomes(partition, placed),
        })
    }

    /// Binds a Bundle whose every entry an earlier delivery committed in
    /// `committed` and did not finish binding, from the contribution read
    /// back.
    ///
    /// Nothing is committed a second time: the versions come from the
    /// CONTRIBUTION the CDR holds, verified entry by entry as a first delivery
    /// verifies them.
    async fn reconcile(
        &self,
        partition: Partition<'_>,
        sources: &[Option<SourceVersion>],
        committed: &CommittedSource,
    ) -> Result<Ingested, Refused> {
        let (ehr_id, contribution) = committed_ids(committed)?;
        let versions = self
            .committed_versions(
                &ehr_id,
                &contribution,
                Returned::Minimal,
                &partition.mapped,
                Scope::Whole,
            )
            .await?;
        let placed = self.place(&partition.mapped, sources, &ehr_id, versions_of(versions))?;
        Ok(Ingested {
            ehr_id,
            commit: Commit::AlreadyConsumed,
            entries: outcomes(partition, placed),
        })
    }

    /// Returns what the identity map records of one Bundle entry's `source`:
    /// the exact source consumed, the contribution that committed it, and,
    /// when neither, the composition another version of its `id` produced.
    ///
    /// Only an entry mapped from a resource has versions of one `id`; the
    /// entries of one message share its control id and are no versions of
    /// each other. The by-id lookup is the one a single create runs
    /// ([`Ingest::recognise`]).
    fn recorded(
        &self,
        source: &SourceVersion,
        provenance: &Provenance,
    ) -> Result<Recorded, Refused> {
        let consumed = self
            .store
            .consumed(source)
            .map_err(|error| store_refusal(&error))?;
        let committed = self
            .store
            .committed(source)
            .map_err(|error| store_refusal(&error))?;
        if consumed.is_some() || committed.is_some() || *provenance != Provenance::EachResource {
            return Ok(Recorded {
                consumed,
                committed,
                revised: None,
            });
        }
        let versions = self
            .store
            .consumed_versions(source)
            .map_err(|error| store_refusal(&error))?;
        Ok(Recorded {
            consumed: None,
            committed: None,
            revised: one_composition(source, versions)?,
        })
    }

    /// Returns the version each mapped entry's commit follows: the latest
    /// version of the composition `revised` names for it, or nothing for an
    /// entry that creates one.
    ///
    /// A revised composition must live in the Bundle's EHR, and one
    /// composition takes one new version per contribution, since each
    /// `UpdateVersion` names the one version it follows
    /// (`ehr-codegen.openapi.yaml`, `UpdateVersion.preceding_version_uid`).
    async fn preceding(
        &self,
        ehr_id: &EhrId,
        mapped: &[Mapped<'_>],
        revised: &[Option<ConsumedSource>],
    ) -> Result<Vec<Option<ObjectVersionId>>, Refused> {
        let mut containers = BTreeSet::new();
        let mut preceding = Vec::with_capacity(revised.len());
        for (entry, revision) in mapped.iter().zip(revised) {
            let Some(known) = revision else {
                preceding.push(None);
                continue;
            };
            if known.ehr_id != ehr_id.as_str() {
                return Err(Refused::new(
                    StatusCode::CONFLICT,
                    Issue::error(IssueType::Conflict)
                        .diagnosing(
                            "the identity map binds this resource to a composition in another EHR than the Bundle's subject",
                        )
                        .at(entry.full_url.clone()),
                ));
            }
            if !containers.insert(known.versioned_object_uid.clone()) {
                return Err(Refused::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Issue::error(IssueType::Duplicate)
                        .diagnosing(
                            "another entry of this transaction revises the same composition, and a contribution carries one version of it",
                        )
                        .at(entry.full_url.clone()),
                ));
            }
            let container = HierObjectId::new(&known.versioned_object_uid)
                .map_err(|error| stored_identifier(&error))?;
            let (latest, _composition) = self
                .read(ehr_id, &UidBasedId::HierObjectId(container))
                .await?;
            let latest = latest.ok_or_else(|| {
                Refused::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Issue::error(IssueType::Exception)
                        .diagnosing("the CDR answered a composition with no ETag"),
                )
            })?;
            preceding.push(Some(latest));
        }
        Ok(preceding)
    }

    /// Records that every keyed entry of a Bundle was committed in
    /// `contribution`, before any of them is bound.
    fn record_commitment(
        &self,
        sources: &[Option<SourceVersion>],
        ehr_id: &EhrId,
        contribution: &ContributionUid,
    ) -> Result<(), Refused> {
        let record = CommittedSource {
            ehr_id: String::from(ehr_id.as_str()),
            contribution_uid: String::from(contribution.as_str()),
        };
        for source in sources.iter().flatten() {
            self.store
                .record_committed(source, &record)
                .map_err(|error| {
                    unbound(
                        contribution,
                        &format!("its commit could not be recorded: {}", chain(&error)),
                    )
                })?;
        }
        Ok(())
    }

    /// Binds and consumes each mapped entry at the version `versions` names
    /// for it, in entry order.
    fn place(
        &self,
        mapped: &[Mapped<'_>],
        sources: &[Option<SourceVersion>],
        ehr_id: &EhrId,
        versions: Vec<ObjectVersionId>,
    ) -> Result<Vec<Placed>, Refused> {
        let mut placed = Vec::with_capacity(versions.len());
        for ((entry, version), source) in mapped.iter().zip(versions).zip(sources) {
            let (id, stood) = self.bind(
                entry.program,
                entry.inbound.resource_type(),
                ehr_id,
                &version,
                &entry.built,
            )?;
            self.index(&entry.inbound, &id)?;
            if let Some(source) = source {
                self.consume(source, &stood, &id)?;
            }
            placed.push(Placed {
                id,
                version,
                change: None,
            });
        }
        Ok(placed)
    }

    /// Returns the version each mapped entry produced in `contribution`, with
    /// the composition the CDR holds for it, in entry order.
    ///
    /// The versions come from the representation the commit answered, or,
    /// when it answered none, from the CONTRIBUTION read back by its uid
    /// (`ehr-codegen.openapi.yaml`, `contribution_get`). Each version is then
    /// read and matched to its entry ([`pair`]) under `scope`.
    async fn committed_versions(
        &self,
        ehr_id: &EhrId,
        contribution: &ContributionUid,
        returned: Returned<Contribution>,
        mapped: &[Mapped<'_>],
        scope: Scope,
    ) -> Result<Vec<(ObjectVersionId, Composition)>, Refused> {
        let stored = match returned {
            Returned::Representation(stored) => *stored,
            Returned::Minimal | Returned::Identifier(_) => {
                self.read_contribution(ehr_id, contribution).await?
            }
        };
        let versions = listed_versions(contribution, &stored, mapped.len(), scope)?;
        let mut read = Vec::with_capacity(versions.len());
        for version in versions {
            let composition = self.version_of(ehr_id, &version).await?;
            read.push((version, composition));
        }
        pair(contribution, mapped, read, scope)
    }

    /// Reads the committed CONTRIBUTION `contribution` back from the CDR.
    async fn read_contribution(
        &self,
        ehr_id: &EhrId,
        contribution: &ContributionUid,
    ) -> Result<Contribution, Refused> {
        let answered = self
            .client
            .contribution(ehr_id, contribution)
            .await
            .map_err(|error| {
                unbound(
                    contribution,
                    &format!("reading it back failed: {}", chain(&error)),
                )
            })?;
        match answered.outcome {
            ContributionGetOutcome::Ok { body, .. } => Ok(body),
            ContributionGetOutcome::NotFound { body } => Err(unbound(
                contribution,
                &format!(
                    "reading it back found nothing: {}",
                    status::diagnostics(StatusCode::NOT_FOUND, &body)
                ),
            )),
        }
    }

    /// Answers a Bundle whose every entry the identity map already consumed,
    /// with each entry where its first delivery put it.
    async fn replay(
        &self,
        partition: Partition<'_>,
        known: &[Option<ConsumedSource>],
    ) -> Result<Ingested, Refused> {
        let mut ehr_id = None;
        let mut placed = Vec::with_capacity(known.len());
        for record in known.iter().flatten() {
            let (ehr, stands) = self.stands(record).await?;
            ehr_id.get_or_insert(ehr);
            placed.push(stands);
        }
        let Some(ehr_id) = ehr_id else {
            return Err(Refused::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                Issue::error(IssueType::Required)
                    .diagnosing("the Bundle carries no entry this server maps"),
            ));
        };
        Ok(Ingested {
            ehr_id,
            commit: Commit::AlreadyConsumed,
            entries: outcomes(partition, placed),
        })
    }

    /// Maps every entry, collecting the refusals rather than stopping at the
    /// first.
    fn map_entries(
        &self,
        entries: &[Value],
        unmapped: UnmappedEntries,
        provenance: &Provenance,
    ) -> Partition<'a> {
        let mut partition = Partition::default();
        let locals: BTreeMap<&str, &Value> = entries
            .iter()
            .filter_map(|entry| Some((entry.get("fullUrl")?.as_str()?, entry.get("resource")?)))
            .collect();
        for (index, entry) in entries.iter().enumerate() {
            let full_url = entry
                .get("fullUrl")
                .and_then(Value::as_str)
                .map_or_else(|| format!("Bundle.entry[{index}]"), str::to_owned);
            let Some(resource) = entry.get("resource") else {
                partition.failures.push(
                    Issue::error(IssueType::Required)
                        .diagnosing("the entry carries no resource")
                        .at(full_url),
                );
                continue;
            };
            match self.map_one(resource, &full_url, index, &locals, provenance) {
                Ok(Mapping::Mapped(one, person)) => {
                    match partition.subject {
                        None => partition.subject = Some(person),
                        Some(ref held) if held == &person => {}
                        Some(_) => {
                            partition.failures.push(
                                Issue::error(IssueType::Processing)
                                    .diagnosing(
                                        "the Bundle references more than one subject, and one Bundle maps to one composition",
                                    )
                                    .at(one.full_url.clone()),
                            );
                        }
                    }
                    partition
                        .pending
                        .push(Pending::Mapped(partition.mapped.len()));
                    partition.mapped.push(*one);
                }
                Ok(Mapping::Unmapped(skipped, issue)) => match unmapped {
                    UnmappedEntries::Refuse => partition.failures.push(issue),
                    UnmappedEntries::SkipAndCount => {
                        partition.pending.push(Pending::Skipped(skipped));
                    }
                },
                Err(issue) => partition.failures.push(issue),
            }
        }
        partition
    }

    /// Maps one Bundle entry, naming it by its `fullUrl` on refusal.
    ///
    /// `locals` holds every entry of the Bundle by its `fullUrl`, so a
    /// subject reference to another entry is read through that entry
    /// ([`local_subject`]).
    fn map_one(
        &self,
        resource: &Value,
        full_url: &str,
        position: usize,
        locals: &BTreeMap<&str, &Value>,
        provenance: &Provenance,
    ) -> Result<Mapping<'a>, Issue> {
        let inbound = Inbound::of(resource.clone(), "").map_err(|error| {
            Issue::error(IssueType::Structure)
                .diagnosing(chain(&error))
                .at(String::from(full_url))
        })?;
        if !self.programs.supports(inbound.resource_type()) {
            return Ok(Mapping::Unmapped(
                Skipped {
                    full_url: String::from(full_url),
                    resource_type: String::from(inbound.resource_type()),
                    reason: SkipReason::NoProgramForType,
                },
                Issue::error(IssueType::NotSupported)
                    .diagnosing(format!(
                        "no loaded mapping answers for {}",
                        inbound.resource_type()
                    ))
                    .at(String::from(full_url)),
            ));
        }
        let program = match self.programs.select(inbound.profiles(), None) {
            Ok(program) => program,
            Err(error) => {
                let issue = Issue::error(IssueType::NotSupported)
                    .diagnosing(error.to_string())
                    .at(String::from(full_url));
                return match error {
                    SelectError::NoMatch { .. } => Ok(Mapping::Unmapped(
                        Skipped {
                            full_url: String::from(full_url),
                            resource_type: String::from(inbound.resource_type()),
                            reason: SkipReason::NoProgramForProfiles {
                                profiles: inbound.profiles().to_vec(),
                            },
                        },
                        issue,
                    )),
                    _ => Err(issue),
                };
            }
        };
        let namespace = self.settings.subject_namespace.as_str();
        let subject = match local_subject(&inbound, locals, namespace) {
            Some(local) => local.map_err(|error| chain(&error)),
            None => inbound.subject(namespace).map_err(|error| chain(&error)),
        }
        .map_err(|text| {
            Issue::error(IssueType::Required)
                .diagnosing(text)
                .at(String::from(full_url))
        })?;
        let (composition, instant) = self.run(program, &inbound, provenance).map_err(|error| {
            Issue::error(IssueType::Processing)
                .diagnosing(chain(&error))
                .at(String::from(full_url))
        })?;
        let text = serde_json::to_string(composition.value()).map_err(|error| {
            Issue::error(IssueType::Exception)
                .diagnosing(format!(
                    "the built composition could not be written: {error}"
                ))
                .at(String::from(full_url))
        })?;
        let rm = openehr_its::json::from_canonical_json::<Composition>(&text).map_err(|error| {
            Issue::error(IssueType::Processing)
                .diagnosing(format!(
                    "the built composition is no valid openEHR COMPOSITION: {error}"
                ))
                .at(String::from(full_url))
        })?;
        Ok(Mapping::Mapped(
            Box::new(Mapped {
                full_url: String::from(full_url),
                position,
                template_id: String::from(composition.template_id()),
                inbound,
                program,
                built: composition,
                composition: Box::new(rm),
                instant,
            }),
            subject,
        ))
    }

    /// Commits every mapped entry as one CONTRIBUTION, and returns its uid
    /// with what the answer carried.
    ///
    /// Each entry goes in as one `UpdateVersion` (`ehr-codegen.openapi.yaml`,
    /// `NewContribution.versions`): a creation when `preceding` names nothing
    /// for it, and otherwise a modification whose `preceding_version_uid` is
    /// the version `preceding` names, so the CDR refuses a stale one.
    ///
    /// A `201` whose body is neither schema the operation admits still named
    /// the committed contribution, so it answers as one that returned nothing
    /// and the versions are read back.
    async fn commit_all(
        &self,
        ehr_id: &EhrId,
        mapped: &[Mapped<'_>],
        preceding: &[Option<ObjectVersionId>],
    ) -> Result<(ContributionUid, Returned<Contribution>), Refused> {
        let system_id = &self.settings.system_id;
        let versions: Vec<UpdateVersion<Versionable>> = mapped
            .iter()
            .zip(preceding)
            .map(|(entry, prior)| UpdateVersion {
                preceding_version_uid: prior.clone(),
                signature: None,
                lifecycle_state: commit::lifecycle(),
                attestations: None,
                data: Versionable::Composition(entry.composition.as_ref().clone()),
                commit_audit: commit::audit(change_of(prior.as_ref()), system_id),
            })
            .collect();
        // NOTE: no specification governs this: our own design; the contribution's
        // own audit states a modification only when every version it carries is one.
        let change = if preceding.iter().all(Option::is_some) {
            commit::Change::Modification
        } else {
            commit::Change::Creation
        };
        let contribution = NewContribution {
            uid: None,
            versions,
            audit: commit::audit(change, system_id),
        };
        let answered = match self
            .client
            .create_contribution(ehr_id, &contribution, Prefer::Representation)
            .await
        {
            Ok(answered) => answered,
            Err(CdrError::CommittedBody {
                contribution_uid, ..
            }) => return Ok((contribution_uid, Returned::Minimal)),
            Err(error) => return Err(cdr_refusal(&error)),
        };
        match answered.outcome {
            ContributionCreateOutcome::Created { body, headers } => {
                let contribution_uid = crate::cdr::contribution_uid_from_etag(
                    "contribution_create",
                    StatusCode::CREATED,
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                let returned = match crate::cdr::committed(
                    &contribution_uid,
                    body.as_ref(),
                    Prefer::Representation,
                ) {
                    Ok(returned) => returned,
                    // NOTE: no specification governs this: our own design; the
                    // commit happened, so its versions are read back by the uid.
                    Err(CdrError::CommittedBody { .. }) => Returned::Minimal,
                    Err(error) => return Err(cdr_refusal(&error)),
                };
                Ok((contribution_uid, returned))
            }
            ContributionCreateOutcome::NoContent { headers } => {
                let contribution_uid = crate::cdr::contribution_uid_from_etag(
                    "contribution_create",
                    StatusCode::NO_CONTENT,
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                Ok((contribution_uid, Returned::Minimal))
            }
            ContributionCreateOutcome::BadRequest { body } => Err(refuse_all(
                mapped,
                &status::BAD_REQUEST,
                &status::diagnostics(StatusCode::BAD_REQUEST, &body),
            )),
            ContributionCreateOutcome::NotFound { body } => Err(refuse_all(
                mapped,
                &status::NOT_FOUND,
                &status::diagnostics(StatusCode::NOT_FOUND, &body),
            )),
            ContributionCreateOutcome::Conflict { body } => Err(refuse_all(
                mapped,
                &status::PRECONDITION_FAILED,
                &status::diagnostics(StatusCode::CONFLICT, &body),
            )),
        }
    }

    /// Runs the engine over one inbound resource.
    fn build(
        &self,
        program: &Loaded,
        inbound: &Inbound,
        provenance: &Provenance,
    ) -> Result<CanonicalComposition, Refused> {
        self.run(program, inbound, provenance)
            .map(|(composition, _instant)| composition)
            .map_err(|error| engine_refusal(&error))
    }

    /// Runs the engine over one inbound resource, and returns the composition
    /// with the instant it defaulted every clock-derived time from.
    fn run(
        &self,
        program: &Loaded,
        inbound: &Inbound,
        provenance: &Provenance,
    ) -> Result<(CanonicalComposition, String), fhirconnect::engine::traverse::EngineError> {
        // NOTE: no specification governs this: our own design, one instant serves
        // every defaulted time of one ingest, so each resource reads the clock once.
        let now = jiff::Timestamp::now().to_string();
        let built = engine::inbound_from(
            program.program(),
            program.index(),
            inbound.document(),
            &now,
            self.settings,
            provenance.source_for(inbound.document()),
        )?;
        Ok((built.into_value(), now))
    }

    /// Returns the commit headers one composition write carries.
    fn commit_context(
        &self,
        change: commit::Change,
        composition: &CanonicalComposition,
    ) -> Result<crate::cdr::commit::CommitContext, Refused> {
        commit::context(change, &self.settings.system_id, composition.template_id()).map_err(
            |error| {
                Refused::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Issue::error(IssueType::Exception).diagnosing(chain(&error)),
                )
            },
        )
    }

    /// Records the identity of a committed write, and returns what stands.
    fn record(
        &self,
        program: &Loaded,
        inbound: &Inbound,
        ehr_id: &EhrId,
        version: &ObjectVersionId,
        composition: &CanonicalComposition,
    ) -> Result<Written, Refused> {
        let resource_type = inbound.resource_type();
        let (id, stood) = self.bind(program, resource_type, ehr_id, version, composition)?;
        self.index(inbound, &id)?;
        if let Some(source) = resource_source(inbound) {
            self.consume(&source, &stood, &id)?;
        }
        Ok(Written {
            id,
            ehr_id: ehr_id.clone(),
            version: version.clone(),
            composition: composition.clone(),
            delivery: Delivery::Committed,
        })
    }

    /// Records the identity binding of one committed composition, and returns
    /// the resource id and the binding that stand.
    fn bind(
        &self,
        program: &Loaded,
        resource_type: &str,
        ehr_id: &EhrId,
        version: &ObjectVersionId,
        composition: &CanonicalComposition,
    ) -> Result<(FhirResourceId, CompositionBinding), Refused> {
        let entry = engine::entry_of(program.program(), composition).ok_or_else(|| {
            Refused::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception).diagnosing(
                    "the committed composition carries no entry of the archetype the program maps",
                ),
            )
        })?;
        // NOTE: no specification governs this: our own design, the facade serves the
        // first resource of a split and carries every further one as contained, so
        // the entry's identity is its first occurrence.
        let key = EntryKey::new(versioned_object_uid(version), entry.path(), 0);
        let map_key = derive::map_key(&key);
        let recorded = self
            .store
            .internal_of(resource_type, &map_key)
            .map_err(|error| store_refusal(&error))?;
        let id = if let Some(known) = recorded {
            known
        } else {
            let (fresh, _source) = derive::derive(&key, entry.uid());
            self.store
                .record_internal(resource_type, &map_key, &fresh)
                .map_err(|error| store_refusal(&error))?
        };
        let binding = CompositionBinding {
            ehr_id: String::from(ehr_id.as_str()),
            versioned_object_uid: String::from(versioned_object_uid(version).value()),
            template_id: String::from(composition.template_id()),
            resource_type: String::from(resource_type),
            entry_path: String::from(entry.path()),
            split: key.split(),
            context: program.program().context().to_string(),
        };
        let stood = self
            .store
            .record_binding(resource_type, &id, &binding)
            .map_err(|error| store_refusal(&error))?;
        Ok((id, stood))
    }

    /// Records every `Resource.identifier` of `inbound` against `id`, so a
    /// conditional create's `identifier` search finds the resource
    /// (<https://hl7.org/fhir/R4/http.html#ccreate>).
    fn index(&self, inbound: &Inbound, id: &FhirResourceId) -> Result<(), Refused> {
        for identifier in identifiers_of(inbound) {
            self.store
                .record_identifier(inbound.resource_type(), &identifier, id)
                .map_err(|error| store_refusal(&error))?;
        }
        Ok(())
    }

    /// Records that `source` was consumed by the composition `stood` binds.
    fn consume(
        &self,
        source: &SourceVersion,
        stood: &CompositionBinding,
        id: &FhirResourceId,
    ) -> Result<(), Refused> {
        self.store
            .record_consumed(
                source,
                &ConsumedSource {
                    ehr_id: stood.ehr_id.clone(),
                    versioned_object_uid: stood.versioned_object_uid.clone(),
                    internal_id: String::from(id.as_str()),
                    context: stood.context.clone(),
                },
            )
            .map_err(|error| store_refusal(&error))?;
        Ok(())
    }

    /// Returns where the composition a consumed source produced stands now.
    ///
    /// The identity map holds the version container and the resource id, and
    /// the CDR holds the version, so the latest one is read from the CDR.
    async fn stands(&self, known: &ConsumedSource) -> Result<(EhrId, Placed), Refused> {
        let standing = self.standing(known).await?;
        Ok((
            standing.ehr_id,
            Placed {
                id: standing.id,
                version: standing.version,
                change: None,
            },
        ))
    }

    /// Returns the composition a consumed source produced, at the version it
    /// stands at now.
    async fn standing(&self, known: &ConsumedSource) -> Result<Standing, Refused> {
        let ehr_id = EhrId::new(&known.ehr_id).map_err(|error| stored_identifier(&error))?;
        let container = HierObjectId::new(&known.versioned_object_uid)
            .map_err(|error| stored_identifier(&error))?;
        let id = FhirResourceId::new(&known.internal_id).map_err(|error| {
            Refused::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception).diagnosing(format!(
                    "the identity map holds a FHIR id this version cannot read: {error}"
                )),
            )
        })?;
        let (version, composition) = self
            .read(&ehr_id, &UidBasedId::HierObjectId(container))
            .await?;
        let version = version.ok_or_else(|| {
            Refused::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception)
                    .diagnosing("the CDR answered a composition with no ETag"),
            )
        })?;
        Ok(Standing {
            ehr_id,
            id,
            version,
            composition,
        })
    }

    /// Returns the composition the CDR holds as `version`.
    async fn version_of(
        &self,
        ehr_id: &EhrId,
        version: &ObjectVersionId,
    ) -> Result<Composition, Refused> {
        self.read(ehr_id, &UidBasedId::ObjectVersionId(version.clone()))
            .await
            .map(|(_version, composition)| composition)
    }

    /// Reads one composition, and returns it with the version its `ETag`
    /// names.
    async fn read(
        &self,
        ehr_id: &EhrId,
        uid: &UidBasedId,
    ) -> Result<(Option<ObjectVersionId>, Composition), Refused> {
        let answered = self
            .client
            .composition(ehr_id, uid, None)
            .await
            .map_err(|error| cdr_refusal(&error))?;
        match answered.outcome {
            CompositionGetOutcome::Ok { body, headers } => {
                let version = crate::cdr::optional_version_from_etag(
                    "composition_get",
                    headers.etag.as_deref(),
                )
                .map_err(|error| cdr_refusal(&error))?;
                Ok((version, body))
            }
            CompositionGetOutcome::NoContent => Err(Refused::of_answer(&status::Answer::new(
                status::GONE,
                String::from("the CDR reports this composition deleted"),
            ))),
            CompositionGetOutcome::NotFound { body } => Err(upstream_refusal(
                status::NOT_FOUND,
                StatusCode::NOT_FOUND,
                &body,
            )),
        }
    }
}

/// Returns the subject a Bundle-local reference of `inbound` names, read from
/// the first `identifier` with a `value` of the entry it references.
///
/// A reference whose value is another entry's `fullUrl` resolves to that
/// entry (R4 `Bundle.entry.fullUrl`; <https://hl7.org/fhir/R4/bundle.html#references>),
/// so a `urn:uuid` a message Bundle allocates per message never becomes a
/// person key of its own. `None` when `inbound` names its subject by
/// identifier, when the reference names no entry of the Bundle, or when that
/// entry carries no identifier with a value; the reference is then read as
/// [`Inbound::subject`] reads it.
fn local_subject(
    inbound: &Inbound,
    locals: &BTreeMap<&str, &Value>,
    namespace: &str,
) -> Option<Result<PersonId, IdError>> {
    let document = inbound.document();
    let reference = document
        .get("subject")
        .or_else(|| document.get("patient"))?;
    if reference.get("identifier").is_some() {
        return None;
    }
    let target = locals.get(reference.get("reference")?.as_str()?)?;
    // NOTE: no specification governs this: our own design; the first identifier
    // in document order keys the person, as PID-3's first repetition does.
    let identifier = target
        .get("identifier")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .find(|identifier| identifier.get("value").and_then(Value::as_str).is_some())?;
    let value = identifier.get("value").and_then(Value::as_str)?;
    let system = identifier
        .get("system")
        .and_then(Value::as_str)
        .unwrap_or(namespace);
    Some(PersonId::new(system, value))
}

/// Returns the refusal of a Bundle no entry of which mapped, naming every
/// entry that was skipped and why.
fn nothing_mapped(partition: &Partition<'_>) -> Refused {
    let mut issues = vec![
        Issue::error(IssueType::Required)
            .diagnosing("the Bundle carries no entry this server maps"),
    ];
    for slot in &partition.pending {
        if let Pending::Skipped(skipped) = slot {
            let why = match skipped.reason {
                SkipReason::NoProgramForType => {
                    format!("no loaded mapping answers for {}", skipped.resource_type)
                }
                SkipReason::NoProgramForProfiles { ref profiles } => format!(
                    "no loaded mapping for {} claims one of the profiles [{}]",
                    skipped.resource_type,
                    profiles.join(", ")
                ),
            };
            issues.push(
                Issue::error(IssueType::NotSupported)
                    .diagnosing(why)
                    .at(skipped.full_url.clone()),
            );
        }
    }
    Refused::of(StatusCode::UNPROCESSABLE_ENTITY, issues)
}

/// Returns the consumed-source key of one resource: its `resourceType`, `id`
/// and `meta.versionId`, or nothing when it has no `id`.
fn resource_source(inbound: &Inbound) -> Option<SourceVersion> {
    inbound.id().map(|external| {
        SourceVersion::new(
            inbound.resource_type(),
            external.clone(),
            inbound.version_id().map(str::to_owned),
        )
    })
}

/// Returns every `Resource.identifier` of `inbound` that carries a `value`,
/// in document order.
///
/// An `identifier` search matches on the value
/// (<https://hl7.org/fhir/R4/search.html#token>), so an `Identifier` with no
/// `value` can answer no search and has nothing to record.
fn identifiers_of(inbound: &Inbound) -> Vec<Identifier> {
    inbound
        .document()
        .get("identifier")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|identifier| {
            let value = identifier.get("value").and_then(Value::as_str)?;
            let system = identifier.get("system").and_then(Value::as_str);
            Some(Identifier::new(system, value))
        })
        .collect()
}

/// Returns the change a version that follows `preceding` makes: a creation
/// when it follows nothing, a modification otherwise.
const fn change_of(preceding: Option<&ObjectVersionId>) -> commit::Change {
    match preceding {
        None => commit::Change::Creation,
        Some(_) => commit::Change::Modification,
    }
}

/// Returns the keys one Bundle delivery claims.
///
/// Every keyed entry claims its source key ([`SourceVersion::storage_key`]).
/// An entry mapped from a resource also claims its `resourceType` and `id`
/// alone ([`SourceVersion::resource_key`]), as a single create does, since a
/// new version of that `id` revises the composition any other version
/// produced; the entries of one message share their message's key, so they
/// claim theirs alone.
fn claim_keys(sources: &[Option<SourceVersion>], provenance: &Provenance) -> Vec<String> {
    let each = *provenance == Provenance::EachResource;
    sources
        .iter()
        .flatten()
        .flat_map(|source| {
            let resource = each.then(|| source.resource_key());
            std::iter::once(source.storage_key()).chain(resource)
        })
        .collect()
}

/// Returns the EHR and the contribution a commit record names.
fn committed_ids(committed: &CommittedSource) -> Result<(EhrId, ContributionUid), Refused> {
    let ehr_id = EhrId::new(&committed.ehr_id).map_err(|error| stored_identifier(&error))?;
    let contribution = ContributionUid::new(&committed.contribution_uid)
        .map_err(|error| stored_identifier(&error))?;
    Ok((ehr_id, contribution))
}

/// Returns the versions of `paired`, in order.
fn versions_of(paired: Vec<(ObjectVersionId, Composition)>) -> Vec<ObjectVersionId> {
    paired.into_iter().map(|(version, _)| version).collect()
}

/// Returns the consumed-source key of each mapped entry, in order.
///
/// An entry mapped from a resource is keyed by its `resourceType`, `id` and
/// `meta.versionId`, as a single create is; an entry of a message is keyed by
/// the message and its position ([`SourceVersion::of_message_entry`]). An
/// entry with no id has no key, so the Bundle it rides in cannot be
/// recognised when it is sent again.
fn sources_of(
    mapped: &[Mapped<'_>],
    provenance: &Provenance,
) -> Result<Vec<Option<SourceVersion>>, Refused> {
    mapped
        .iter()
        .map(|entry| match provenance {
            Provenance::EachResource => Ok(resource_source(&entry.inbound)),
            Provenance::Item(item) => {
                let Some(control) = item.id() else {
                    return Ok(None);
                };
                ExternalResourceId::new(control)
                    .and_then(|control| {
                        SourceVersion::of_message_entry(
                            item.resource_type(),
                            control,
                            entry.position,
                        )
                    })
                    .map(Some)
                    .map_err(|error| {
                        Refused::new(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            Issue::error(IssueType::Value)
                                .diagnosing(format!(
                                    "the message cannot key the identity map: {error}"
                                ))
                                .at(entry.full_url.clone()),
                        )
                    })
            }
        })
        .collect()
}

/// Refuses a Bundle in which two entries carry one source key.
///
/// "A resource can only appear in a transaction once (by identity)"
/// (<https://hl7.org/fhir/R4/http.html#transaction>), and one key would bind
/// two compositions to one consumed source.
fn distinct(mapped: &[Mapped<'_>], sources: &[Option<SourceVersion>]) -> Result<(), Refused> {
    let mut seen = BTreeSet::new();
    let issues: Vec<Issue> = mapped
        .iter()
        .zip(sources)
        .filter_map(|(entry, source)| {
            let source = source.as_ref()?;
            if seen.insert(source.storage_key()) {
                return None;
            }
            Some(
                Issue::error(IssueType::Duplicate)
                    .diagnosing(format!(
                        "{}/{} appears in this transaction more than once",
                        source.resource_type(),
                        source.id()
                    ))
                    .at(entry.full_url.clone()),
            )
        })
        .collect();
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Refused::of(StatusCode::UNPROCESSABLE_ENTITY, issues))
    }
}

/// Returns the refusal of a delivery that commits a source its claim does
/// not hold.
fn unclaimed() -> Refused {
    Refused::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception)
            .diagnosing("the delivery holds no claim on the source it commits"),
    )
}

/// Returns the one composition every consumed version of `source`'s `id`
/// names, or nothing when the map consumed none.
///
/// Two versions of one `id` that name two compositions leave no one
/// composition a later version revises, so the create is refused with the
/// record intact (no specification governs this: our own design).
fn one_composition(
    source: &SourceVersion,
    versions: Vec<ConsumedSource>,
) -> Result<Option<ConsumedSource>, Refused> {
    let mut versions = versions.into_iter();
    let Some(first) = versions.next() else {
        return Ok(None);
    };
    let split = versions.any(|other| {
        other.ehr_id != first.ehr_id || other.versioned_object_uid != first.versioned_object_uid
    });
    if split {
        return Err(Refused::new(
            StatusCode::CONFLICT,
            Issue::error(IssueType::Conflict)
                .diagnosing(
                    "the identity map binds versions of this resource to more than one composition, so none is the one this version revises",
                )
                .at(format!("{}/{}", source.resource_type(), source.id())),
        ));
    }
    Ok(Some(first))
}

/// Returns the refusal of a delivery whose source another in-flight delivery
/// holds, naming each place in `at`.
///
/// Nothing is committed; the caller retries once the first delivery settles
/// (no specification governs the redelivery rule: our own design).
fn in_flight(at: impl IntoIterator<Item = String>) -> Refused {
    let mut issues = vec![Issue::error(IssueType::Duplicate).diagnosing(
        "another delivery of the same source is in flight, so nothing is committed; retry once it settles",
    )];
    issues.extend(at.into_iter().map(|place| {
        Issue::error(IssueType::Duplicate)
            .diagnosing("another in-flight delivery holds this entry")
            .at(place)
    }));
    Refused::of(StatusCode::CONFLICT, issues)
}

/// Returns the refusal of a Bundle whose entries `contended` names another
/// in-flight delivery as holding.
fn contended_entries(
    mapped: &[Mapped<'_>],
    sources: &[Option<SourceVersion>],
    contended: &Contended,
) -> Refused {
    in_flight(
        mapped
            .iter()
            .zip(sources)
            .filter(|(_, source)| {
                source.as_ref().is_some_and(|source| {
                    contended.holds(&source.storage_key())
                        || contended.holds(&source.resource_key())
                })
            })
            .map(|(entry, _)| entry.full_url.clone()),
    )
}

/// Returns the contribution every entry of a Bundle was committed in, when
/// one contribution committed them all.
fn one_contribution(committed: &[Option<CommittedSource>]) -> Option<&CommittedSource> {
    let first = committed.first()?.as_ref()?;
    committed
        .iter()
        .all(|record| record.as_ref() == Some(first))
        .then_some(first)
}

/// Returns the refusal of a Bundle only some of whose entries were consumed.
///
/// A transaction is all or nothing (<https://hl7.org/fhir/R4/http.html#transaction>),
/// so the Bundle neither commits its fresh entries nor answers the consumed
/// ones; the refusal names every entry an earlier delivery consumed or
/// committed, as `seen` marks them.
fn partly_consumed(mapped: &[Mapped<'_>], seen: &[bool]) -> Refused {
    let mut issues = vec![Issue::error(IssueType::Duplicate).diagnosing(
        "an earlier delivery consumed some entries of this Bundle and not the others, so nothing is committed",
    )];
    for (entry, was_seen) in mapped.iter().zip(seen) {
        if *was_seen {
            issues.push(
                Issue::error(IssueType::Duplicate)
                    .diagnosing("an earlier delivery already consumed this entry")
                    .at(entry.full_url.clone()),
            );
        }
    }
    Refused::of(StatusCode::CONFLICT, issues)
}

/// Returns one outcome per entry, in Bundle order, each mapped entry at the
/// place `placed` gives it.
fn outcomes(partition: Partition<'_>, placed: Vec<Placed>) -> Vec<EntryOutcome> {
    let mut committed: Vec<Option<Committed>> = partition
        .mapped
        .iter()
        .zip(placed)
        .map(|(entry, place)| {
            Some(Committed {
                full_url: entry.full_url.clone(),
                resource_type: String::from(entry.inbound.resource_type()),
                template_id: entry.template_id.clone(),
                id: place.id,
                version: place.version,
                change: place.change,
            })
        })
        .collect();
    // NOTE: no specification governs this: our own design; `map_entries` hands
    // each mapped index out once and every caller places each mapped entry, so
    // a slot that finds nothing cannot occur.
    partition
        .pending
        .into_iter()
        .filter_map(|slot| match slot {
            Pending::Skipped(skipped) => Some(EntryOutcome::Skipped(skipped)),
            Pending::Mapped(index) => committed
                .get_mut(index)
                .and_then(Option::take)
                .map(EntryOutcome::Committed),
        })
        .collect()
}

/// Returns the refusal of a committed contribution whose answer cannot bind
/// its entries, naming the contribution so it can be reconciled.
fn unbound(contribution: &ContributionUid, why: &str) -> Refused {
    Refused::of_answer(&status::Answer::new(
        status::UNDOCUMENTED,
        format!(
            "the CDR committed contribution {contribution} and {why}, so its entries cannot be bound to their compositions"
        ),
    ))
}

/// What names the item one composition was mapped from: the
/// `originating_system_item_ids[0]` type and id and the
/// `originating_system_audit.version_id` the engine writes into its
/// `FEEDER_AUDIT`
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_feeder_audit_class>).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AuditKey {
    /// The item's type.
    item_type: Option<String>,
    /// The item's id.
    item_id: Option<String>,
    /// The item's version.
    version_id: Option<String>,
}

impl AuditKey {
    /// Returns the key the `FEEDER_AUDIT` of `composition` states.
    fn of(composition: &Composition) -> Self {
        let audit = composition.feeder_audit.as_ref();
        let item = audit
            .and_then(|audit| audit.originating_system_item_ids.as_deref())
            .and_then(<[_]>::first);
        Self {
            item_type: item.and_then(|item| item.r#type.clone()),
            item_id: item.map(|item| item.id.clone()),
            version_id: audit.and_then(|audit| audit.originating_system_audit.version_id.clone()),
        }
    }
}

/// Returns the JSON pointer of every `DV_DATE_TIME` in `composition` whose
/// value is `instant`, the clock reading the run defaulted from.
///
/// The engine hands its one instant to the Simplified Formats builder as the
/// `ctx/time` default, which fills `EVENT_CONTEXT.start_time`,
/// `HISTORY.origin`, `EVENT.time` and `ACTION.time` wherever no mapping wrote
/// them (master06 §time), and the builder copies the instant verbatim, so a
/// value equal to it is one the clock supplied.
fn clock_filled(composition: &serde_json::Value, instant: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(String::new(), composition)];
    while let Some((pointer, node)) = pending.pop() {
        match node {
            serde_json::Value::Object(members) => {
                let is_instant = members.get("_type").and_then(serde_json::Value::as_str)
                    == Some("DV_DATE_TIME")
                    && members.get("value").and_then(serde_json::Value::as_str) == Some(instant);
                if is_instant {
                    found.push(pointer);
                    continue;
                }
                for (name, member) in members {
                    let step = name.replace('~', "~0").replace('/', "~1");
                    pending.push((format!("{pointer}/{step}"), member));
                }
            }
            serde_json::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    pending.push((format!("{pointer}/{index}"), item));
                }
            }
            _ => {}
        }
    }
    found.sort();
    found
}

/// Whether the composition `sent`, mapped at `instant`, and a stored one
/// carry the same content.
///
/// The comparison masks what differs between two mappings of one resource and
/// is no content of the source: the `uid` the CDR assigns, and every time the
/// run filled from its clock ([`clock_filled`]), whatever the stored
/// composition holds at that place.
fn same_content(sent: &Composition, instant: &str, stored: &Composition) -> bool {
    let canonical = |composition: &Composition| {
        let mut composition = composition.clone();
        composition.uid = None;
        openehr_its::json::to_canonical_json(&composition).parse::<serde_json::Value>()
    };
    // NOTE: no specification governs this: our own design; a composition that
    // does not read back as JSON cannot be compared, so it matches nothing.
    let (Ok(mut sent), Ok(mut held)) = (canonical(sent), canonical(stored)) else {
        return false;
    };
    for pointer in clock_filled(&sent, instant) {
        for tree in [&mut sent, &mut held] {
            if let Some(slot) = tree.pointer_mut(&pointer) {
                *slot = serde_json::Value::Null;
            }
        }
    }
    sent == held
}

/// Returns the entry each committed version is the composition of, one
/// version per entry with its stored composition, in entry order.
///
/// `stored` is each version the contribution named with the composition the
/// CDR holds for it. A version matches an entry when its `FEEDER_AUDIT` names
/// the entry's item ([`AuditKey`]); where several entries name one item, as
/// the entries of one message do, it must also carry the entry's content
/// ([`same_content`]: the CDR-assigned `uid` and every time the run filled
/// from its clock are masked, so a re-sent Bundle mapped at another instant
/// still matches). A version that matches more than one entry, or an entry
/// two versions match or none, refuses the whole answer, and so does a
/// version that matches no entry under [`Scope::Whole`]. No specification
/// governs the matching: our own design.
fn pair(
    contribution: &ContributionUid,
    mapped: &[Mapped<'_>],
    stored: Vec<(ObjectVersionId, Composition)>,
    scope: Scope,
) -> Result<Vec<(ObjectVersionId, Composition)>, Refused> {
    let keys: Vec<AuditKey> = mapped
        .iter()
        .map(|entry| AuditKey::of(&entry.composition))
        .collect();
    let mut placed: Vec<Option<(ObjectVersionId, Composition)>> = vec![None; mapped.len()];
    for (version, composition) in stored {
        let key = AuditKey::of(&composition);
        let sharing = keys.iter().filter(|candidate| **candidate == key).count();
        let matching: Vec<usize> = mapped
            .iter()
            .zip(&keys)
            .enumerate()
            .filter(|(_, (entry, candidate))| {
                **candidate == key
                    && (sharing == 1
                        || same_content(&entry.composition, &entry.instant, &composition))
            })
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() && scope == Scope::Part {
            continue;
        }
        let [index] = matching.as_slice() else {
            return Err(unbound(
                contribution,
                &format!(
                    "its version {} matches {} of the entries sent, where it must match one",
                    version.value(),
                    matching.len()
                ),
            ));
        };
        let slot = placed.get_mut(*index).ok_or_else(|| {
            unbound(
                contribution,
                &format!("its version {} matches no entry", version.value()),
            )
        })?;
        if slot.is_some() {
            let full_url = mapped
                .get(*index)
                .map_or("an entry", |entry| entry.full_url.as_str());
            return Err(unbound(
                contribution,
                &format!("two of its versions match {full_url}"),
            ));
        }
        *slot = Some((version, composition));
    }
    placed
        .into_iter()
        .zip(mapped)
        .map(|(slot, entry)| {
            slot.ok_or_else(|| {
                unbound(
                    contribution,
                    &format!("none of its versions matches {}", entry.full_url),
                )
            })
        })
        .collect()
}

/// Returns the versions a committed contribution names.
///
/// The CONTRIBUTION's `versions` reference every version it committed
/// (`ehr-codegen.openapi.yaml`, `components.schemas.Contribution`). A version
/// list shorter than the entries, or under [`Scope::Whole`] of any other
/// length, cannot bind the entries, and the refusal names the contribution so it can be reconciled. The list states no
/// order, so [`pair`] matches each version to its entry.
fn listed_versions(
    contribution: &ContributionUid,
    stored: &Contribution,
    expected: usize,
    scope: Scope,
) -> Result<Vec<ObjectVersionId>, Refused> {
    let unreadable = |why: String| unbound(contribution, &why);
    let references: &[ObjectRef] = stored.versions.as_ref();
    let fits = match scope {
        Scope::Whole => references.len() == expected,
        Scope::Part => references.len() >= expected,
    };
    if !fits {
        return Err(unreadable(format!(
            "names {} versions for {expected} entries",
            references.len()
        )));
    }
    references
        .iter()
        .map(|reference| {
            let ObjectRef::ObjectRef(data) = reference else {
                return Err(unreadable(String::from(
                    "references a version by a reference that is no OBJECT_REF",
                )));
            };
            let ObjectId::ObjectVersionId(ref named) = data.id else {
                return Err(unreadable(String::from(
                    "references a version by an id that is no OBJECT_VERSION_ID",
                )));
            };
            Ok(named.clone())
        })
        .collect()
}

/// Returns the refusal a malformed stored openEHR identifier renders as.
fn stored_identifier(error: &impl std::fmt::Display) -> Refused {
    Refused::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception).diagnosing(format!(
            "the identity map holds an openEHR identifier this version cannot read: {error}"
        )),
    )
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

/// Re-reads a built composition through the strict RM reader.
///
/// The built composition is re-read before it is sent, so a bad document never
/// reaches the CDR. It already carries the `FEEDER_AUDIT` the engine wrote
/// ([`engine::inbound_from`]), so the CDR stores where the content came from.
fn strict_read(composition: &CanonicalComposition) -> Result<Composition, Refused> {
    let text = serde_json::to_string(composition.value()).map_err(|error| {
        Refused::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(format!(
                "the built composition could not be written: {error}"
            )),
        )
    })?;
    openehr_its::json::from_canonical_json::<Composition>(&text).map_err(|error| {
        Refused::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Processing).diagnosing(format!(
                "the built composition is no valid openEHR COMPOSITION: {error}"
            )),
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

/// Returns the composition the CDR stored, or the built one.
///
/// The CDR is the authority on what it stored, so its representation wins when
/// `Prefer: return=representation` produced one, and the built composition
/// stands only when the CDR returned none.
///
/// # Errors
///
/// Returns a [`Refused`] carrying [`RepresentationError::Unreadable`] when the
/// returned composition does not read back. It answers the status table's
/// [`status::INTERNAL`] row, as an undecodable CDR body does through
/// [`status::of_client_error`].
fn representation(
    returned: Returned<Composition>,
    built: &CanonicalComposition,
) -> Result<CanonicalComposition, Refused> {
    match returned {
        Returned::Representation(stored) => {
            let value = openehr_its::json::to_canonical_json(stored.as_ref())
                .parse::<serde_json::Value>()
                .map_err(|source| {
                    let error = RepresentationError::Unreadable { source };
                    Refused::of_answer(&status::Answer::new(status::INTERNAL, chain(&error)))
                })?;
            Ok(CanonicalComposition::new(
                value,
                built.template_id(),
                built.generation(),
            ))
        }
        Returned::Minimal | Returned::Identifier(_) => Ok(built.clone()),
    }
}

/// Returns the refusal one engine error renders as.
///
/// An element the program cannot map refuses the unit (no specification
/// governs this: our own design), so the answer is a `422` whose diagnostics
/// name the mapping and the element the engine refused at.
#[must_use]
pub fn engine_refusal(error: &fhirconnect::engine::traverse::EngineError) -> Refused {
    Refused::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        Issue::error(IssueType::Processing)
            .diagnosing(chain(error))
            .detailing("the mapping refused this resource"),
    )
}

/// Returns the refusal a CDR call that reached no usable documented answer
/// renders as, through the status table.
fn cdr_refusal(error: &CdrError) -> Refused {
    Refused::of_answer(&status::of_client_error(error))
}

/// Returns the refusal a documented CDR `answered` refusal with `body` decides
/// through `row`.
fn upstream_refusal(
    row: status::Row,
    answered: StatusCode,
    body: &openehr_its::rest::client::ErrorBody,
) -> Refused {
    Refused::of_answer(&status::Answer::new(
        row,
        status::diagnostics(answered, body),
    ))
}

/// Returns the refusal a rejected contribution renders as.
///
/// Nothing was committed, so the answer names every entry the Bundle mapped:
/// a caller cannot tell from a partial list which entries still stand.
fn refuse_all(mapped: &[Mapped<'_>], row: &status::Row, detail: &str) -> Refused {
    let mut issues = vec![
        Issue::error(row.issue())
            .diagnosing(detail.to_owned())
            .detailing("the CDR refused the contribution, so no entry of this Bundle is stored"),
    ];
    for entry in mapped {
        issues.push(
            Issue::error(row.issue())
                .diagnosing(detail.to_owned())
                .at(entry.full_url.clone()),
        );
    }
    Refused::of(row.status(), issues)
}

/// Returns the refusal an EHR resolution of a single write renders as.
fn ehr_refusal(error: &ehr::EhrError) -> Refused {
    match *error {
        ehr::EhrError::Absent { .. } => Refused::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::NotFound).diagnosing(chain(error)),
        ),
        ehr::EhrError::Refused { .. } | ehr::EhrError::Subject { .. } => Refused::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            Issue::error(IssueType::Processing).diagnosing(chain(error)),
        ),
        ehr::EhrError::Client { ref source } => {
            Refused::of_answer(&status::of_client_error(source))
        }
        _ => Refused::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            Issue::error(IssueType::Exception).diagnosing(chain(error)),
        ),
    }
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
    fn the_clock_mask_names_every_date_time_at_the_instant_and_nothing_else() {
        let instant = "2026-09-25T10:00:00.123456789Z";
        let composition = serde_json::json!({
            "_type": "COMPOSITION",
            "context": {
                "_type": "EVENT_CONTEXT",
                "start_time": { "_type": "DV_DATE_TIME", "value": instant }
            },
            "content": [{
                "_type": "OBSERVATION",
                "data": {
                    "_type": "HISTORY",
                    "origin": { "_type": "DV_DATE_TIME", "value": instant },
                    "events": [{
                        "_type": "POINT_EVENT",
                        "time": { "_type": "DV_DATE_TIME", "value": "2026-09-13T10:00:00+02:00" }
                    }]
                },
                "name": { "_type": "DV_TEXT", "value": instant }
            }]
        });
        assert_eq!(
            vec![
                String::from("/content/0/data/origin"),
                String::from("/context/start_time"),
            ],
            super::clock_filled(&composition, instant),
            "a mapped time and a text that reads like the instant stay compared"
        );
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
