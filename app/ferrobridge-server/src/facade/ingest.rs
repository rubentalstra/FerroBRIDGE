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

use std::collections::BTreeSet;

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::composition::CompositionOutcome;
use ferrobridge_openehr::composition::CreateCompositionOutcome;
use ferrobridge_openehr::composition::UpdateCompositionOutcome;
use ferrobridge_openehr::contribution::ContributionOutcome;
use ferrobridge_openehr::contribution::CreateContributionOutcome;
use ferrobridge_openehr::ids::ContributionUid;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::ids::versioned_object_uid;
use ferrobridge_openehr::prefer::Prefer;
use ferrobridge_openehr::prefer::Returned;
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
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::common::change_control::contribution::Contribution;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::facade::Settings;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::engine;
use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::PersonId;
use crate::facade::identity::derive;
use crate::facade::identity::derive::EntryKey;
use crate::facade::identity::record::CommittedSource;
use crate::facade::identity::record::CompositionBinding;
use crate::facade::identity::record::ConsumedSource;
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
    /// The CDR client every call of this ingest goes through.
    client: Client,
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
}

impl<'a> Ingest<'a> {
    /// Returns an ingest over `programs`, `store`, `client` and `settings`.
    #[must_use]
    pub const fn new(
        programs: &'a Programs,
        store: &'a dyn Store,
        client: Client,
        settings: &'a Settings,
    ) -> Self {
        Self {
            programs,
            store,
            client,
            settings,
        }
    }

    /// Returns the CDR client this ingest calls through.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// Returns what the identity map recorded for this source resource
    /// version.
    ///
    /// # Errors
    ///
    /// Returns a `500` [`Refused`] when the identity store cannot be read.
    pub fn consumed(&self, inbound: &Inbound) -> Result<Option<ConsumedSource>, Refused> {
        let Some(external) = inbound.id() else {
            return Ok(None);
        };
        let source = SourceVersion::new(
            inbound.resource_type(),
            external.clone(),
            inbound.version_id().map(str::to_owned),
        );
        self.store
            .consumed(&source)
            .map_err(|error| store_refusal(&error))
    }

    /// Commits the first version of a composition for `inbound`.
    ///
    /// The EHR is the subject's, resolved or created under the configured
    /// policy; the identity binding and the consumed source version are
    /// recorded once the CDR stored the composition.
    ///
    /// # Errors
    ///
    /// Returns a [`Refused`] for a resource that names no usable subject, an
    /// EHR that cannot be resolved, a resource the program cannot map, a
    /// composition the strict reader refuses, every CDR refusal through the
    /// status table, and an identity store that cannot be written.
    pub async fn ingest_resource(
        &self,
        inbound: &Inbound,
        program: &Loaded,
        provenance: &Provenance,
    ) -> Result<Written, Refused> {
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
            .map_err(|error| Refused::of_answer(&status::of_client_error(&error)))?;
        let (version, stored) = match answered {
            CreateCompositionOutcome::Created {
                version_id,
                returned,
            } => (version_id, representation(returned, &composition)?),
            CreateCompositionOutcome::Unprocessable(upstream) => {
                return Err(upstream_refusal(status::UNPROCESSABLE, &upstream));
            }
            CreateCompositionOutcome::UnknownEhr(upstream) => {
                return Err(upstream_refusal(status::NOT_FOUND, &upstream));
            }
            CreateCompositionOutcome::BadRequest(upstream) => {
                return Err(upstream_refusal(status::BAD_REQUEST, &upstream));
            }
            _ => return Err(Refused::of_answer(&status::unread("composition_create"))),
        };
        self.record(program, inbound, &ehr_id, &version, &stored)
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
            .map_err(|error| Refused::of_answer(&status::of_client_error(&error)))?;
        let (version, stored) = match answered {
            UpdateCompositionOutcome::Updated {
                version_id,
                returned,
            } => (version_id, representation(returned, &composition)?),
            UpdateCompositionOutcome::Unprocessable(upstream) => {
                return Err(upstream_refusal(status::UNPROCESSABLE, &upstream));
            }
            UpdateCompositionOutcome::PreconditionFailed {
                latest_version_id,
                upstream,
            } => {
                let mut answer = status::Answer::new(
                    status::PRECONDITION_FAILED,
                    status::diagnostics(&upstream),
                );
                if let Some(latest) = latest_version_id {
                    answer = answer.with_entity_tag(String::from(latest.version_tree_id().value()));
                }
                return Err(Refused::of_answer(&answer));
            }
            UpdateCompositionOutcome::NotFound(upstream) => {
                return Err(upstream_refusal(status::NOT_FOUND, &upstream));
            }
            UpdateCompositionOutcome::BadRequest(upstream) => {
                return Err(upstream_refusal(status::BAD_REQUEST, &upstream));
            }
            _ => return Err(Refused::of_answer(&status::unread("composition_update"))),
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
    /// The commit is two steps. The contribution uid the CDR answers is
    /// recorded against every keyed entry ([`CommittedSource`]) before any
    /// entry is bound; the versions then come from the answer's CONTRIBUTION
    /// or, when it carries none, from the CONTRIBUTION read back by that uid.
    /// A Bundle whose every entry one recorded contribution committed is bound
    /// from that contribution read back, commits nothing and answers
    /// [`Commit::AlreadyConsumed`], so a retry after a failed binding is safe.
    /// No specification governs the redelivery rule: our own design.
    ///
    /// # Errors
    ///
    /// Returns a `422` [`Refused`] naming every entry that does not map (and,
    /// under [`UnmappedEntries::Refuse`], every entry no program maps), a
    /// second subject, two entries with one source key, a Bundle with nothing
    /// to commit, and an EHR that cannot be resolved; a `409` naming every
    /// entry already consumed or committed when not all were; the CDR's
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
            return Err(Refused::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                Issue::error(IssueType::Required)
                    .diagnosing("the Bundle carries no entry this server maps"),
            ));
        };
        let sources = sources_of(&partition.mapped, provenance)?;
        distinct(&partition.mapped, &sources)?;
        let mut known = Vec::with_capacity(sources.len());
        let mut committed = Vec::with_capacity(sources.len());
        for source in &sources {
            let (recorded, contribution) = match source {
                Some(source) => (
                    self.store
                        .consumed(source)
                        .map_err(|error| store_refusal(&error))?,
                    self.store
                        .committed(source)
                        .map_err(|error| store_refusal(&error))?,
                ),
                None => (None, None),
            };
            known.push(recorded);
            committed.push(contribution);
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
            .map_err(|error| {
                Refused::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Issue::error(IssueType::Processing).diagnosing(chain(&error)),
                )
            })?;
        let (contribution, returned) = self.commit_all(&ehr_id, &partition.mapped).await?;
        self.record_commitment(&sources, &ehr_id, &contribution)?;
        let versions = self
            .committed_versions(&ehr_id, &contribution, returned, &partition.mapped)
            .await?;
        let placed = self.place(&partition.mapped, &sources, &ehr_id, versions)?;
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
        let ehr_id = EhrId::new(&committed.ehr_id).map_err(|error| stored_identifier(&error))?;
        let contribution = ContributionUid::new(&committed.contribution_uid)
            .map_err(|error| stored_identifier(&error))?;
        let versions = self
            .committed_versions(&ehr_id, &contribution, Returned::Minimal, &partition.mapped)
            .await?;
        let placed = self.place(&partition.mapped, sources, &ehr_id, versions)?;
        Ok(Ingested {
            ehr_id,
            commit: Commit::AlreadyConsumed,
            entries: outcomes(partition, placed),
        })
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
            if let Some(source) = source {
                self.consume(source, &stood, &id)?;
            }
            placed.push(Placed { id, version });
        }
        Ok(placed)
    }

    /// Returns the version each mapped entry produced in `contribution`, in
    /// entry order.
    ///
    /// The versions come from the representation the commit answered, or,
    /// when it answered none, from the CONTRIBUTION read back by its uid
    /// (`ehr-codegen.openapi.yaml`, `contribution_get`). Each version is then
    /// read and matched to its entry ([`pair`]).
    async fn committed_versions(
        &self,
        ehr_id: &EhrId,
        contribution: &ContributionUid,
        returned: Returned<Contribution>,
        mapped: &[Mapped<'_>],
    ) -> Result<Vec<ObjectVersionId>, Refused> {
        let stored = match returned {
            Returned::Representation(stored) => *stored,
            Returned::Minimal | Returned::Identifier(_) => {
                self.read_contribution(ehr_id, contribution).await?
            }
        };
        let versions = listed_versions(contribution, &stored, mapped.len())?;
        let mut read = Vec::with_capacity(versions.len());
        for version in versions {
            let composition = self.version_of(ehr_id, &version).await?;
            read.push((version, composition));
        }
        pair(contribution, mapped, read)
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
        match answered {
            ContributionOutcome::Found(stored) => Ok(*stored),
            ContributionOutcome::NotFound(upstream) => Err(unbound(
                contribution,
                &format!(
                    "reading it back found nothing: {}",
                    status::diagnostics(&upstream)
                ),
            )),
            _ => Err(unbound(
                contribution,
                "reading it back answered what this version cannot read",
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
            match self.map_one(resource, &full_url, index, provenance) {
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
    fn map_one(
        &self,
        resource: &Value,
        full_url: &str,
        position: usize,
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
        let subject = inbound
            .subject(&self.settings.subject_namespace)
            .map_err(|error| {
                Issue::error(IssueType::Required)
                    .diagnosing(chain(&error))
                    .at(String::from(full_url))
            })?;
        // NOTE: no specification governs this: our own design, one instant serves
        // every defaulted time of one ingest, so each entry reads the clock once.
        let now = jiff::Timestamp::now().to_string();
        let built = engine::inbound_from(
            program.program(),
            program.index(),
            inbound.document(),
            &now,
            self.settings,
            provenance.source_for(inbound.document()),
        )
        .map_err(|error| {
            Issue::error(IssueType::Processing)
                .diagnosing(chain(&error))
                .at(String::from(full_url))
        })?;
        let composition = built.into_value();
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
            }),
            subject,
        ))
    }

    /// Commits every mapped entry as one CONTRIBUTION, and returns its uid
    /// with what the answer carried.
    ///
    /// A `201` whose body is neither schema the operation admits still named
    /// the committed contribution, so it answers as one that returned nothing
    /// and the versions are read back.
    async fn commit_all(
        &self,
        ehr_id: &EhrId,
        mapped: &[Mapped<'_>],
    ) -> Result<(ContributionUid, Returned<Contribution>), Refused> {
        let system_id = &self.settings.system_id;
        let contribution = NewContribution {
            uid: None,
            versions: mapped
                .iter()
                .map(|entry| UpdateVersion {
                    preceding_version_uid: None,
                    signature: None,
                    lifecycle_state: commit::lifecycle(),
                    attestations: None,
                    data: Versionable::Composition(entry.composition.as_ref().clone()),
                    commit_audit: commit::audit(commit::Change::Creation, system_id),
                })
                .collect(),
            audit: commit::audit(commit::Change::Creation, system_id),
        };
        let answered = match self
            .client
            .create_contribution(ehr_id, &contribution, Prefer::Representation)
            .await
        {
            Ok(answered) => answered,
            Err(ferrobridge_openehr::error::Error::CommittedBody {
                contribution_uid, ..
            }) => return Ok((contribution_uid, Returned::Minimal)),
            Err(error) => return Err(Refused::of_answer(&status::of_client_error(&error))),
        };
        match answered {
            CreateContributionOutcome::Created {
                contribution_uid,
                returned,
            } => Ok((contribution_uid, returned)),
            CreateContributionOutcome::BadRequest(upstream) => {
                Err(refuse_all(mapped, &status::BAD_REQUEST, &upstream))
            }
            CreateContributionOutcome::UnknownEhr(upstream) => {
                Err(refuse_all(mapped, &status::NOT_FOUND, &upstream))
            }
            CreateContributionOutcome::Conflict(upstream) => {
                Err(refuse_all(mapped, &status::PRECONDITION_FAILED, &upstream))
            }
            _ => Err(Refused::of_answer(&status::unread("contribution_create"))),
        }
    }

    /// Runs the engine over one inbound resource.
    fn build(
        &self,
        program: &Loaded,
        inbound: &Inbound,
        provenance: &Provenance,
    ) -> Result<CanonicalComposition, Refused> {
        // NOTE: no specification governs this: our own design, one instant serves
        // every defaulted time of one ingest, so the clock is read once here.
        let now = jiff::Timestamp::now().to_string();
        engine::inbound_from(
            program.program(),
            program.index(),
            inbound.document(),
            &now,
            self.settings,
            provenance.source_for(inbound.document()),
        )
        .map(fhirconnect::engine::outcome::Outcome::into_value)
        .map_err(|error| engine_refusal(&error))
    }

    /// Returns the commit headers one composition write carries.
    fn commit_context(
        &self,
        change: commit::Change,
        composition: &CanonicalComposition,
    ) -> Result<ferrobridge_openehr::commit::CommitContext, Refused> {
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
        if let Some(external) = inbound.id() {
            let source = SourceVersion::new(
                resource_type,
                external.clone(),
                inbound.version_id().map(str::to_owned),
            );
            self.consume(&source, &stood, &id)?;
        }
        Ok(Written {
            id,
            ehr_id: ehr_id.clone(),
            version: version.clone(),
            composition: composition.clone(),
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
        let (version, _composition) = self
            .read(&ehr_id, &UidBasedId::HierObjectId(container))
            .await?;
        let version = version.ok_or_else(|| {
            Refused::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                Issue::error(IssueType::Exception)
                    .diagnosing("the CDR answered a composition with no ETag"),
            )
        })?;
        Ok((ehr_id, Placed { id, version }))
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
            .map_err(|error| Refused::of_answer(&status::of_client_error(&error)))?;
        match answered {
            CompositionOutcome::Found {
                version_id,
                composition,
            } => Ok((version_id, *composition)),
            CompositionOutcome::Deleted => Err(Refused::of_answer(&status::Answer::new(
                status::GONE,
                String::from("the CDR reports this composition deleted"),
            ))),
            CompositionOutcome::NotFound(upstream) => {
                Err(upstream_refusal(status::NOT_FOUND, &upstream))
            }
            _ => Err(Refused::of_answer(&status::unread("composition_get"))),
        }
    }
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
            Provenance::EachResource => Ok(entry.inbound.id().map(|external| {
                SourceVersion::new(
                    entry.inbound.resource_type(),
                    external.clone(),
                    entry.inbound.version_id().map(str::to_owned),
                )
            })),
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

/// Whether two compositions carry the same content, the CDR-assigned `uid`
/// aside.
fn same_content(sent: &Composition, stored: &Composition) -> bool {
    let mut sent = sent.clone();
    let mut stored = stored.clone();
    sent.uid = None;
    stored.uid = None;
    sent == stored
}

/// Returns the entry each committed version is the composition of, one
/// version per entry, in entry order.
///
/// `stored` is each version the contribution named with the composition the
/// CDR holds for it. A version matches an entry when its `FEEDER_AUDIT` names
/// the entry's item ([`AuditKey`]); where several entries name one item, as
/// the entries of one message do, it must also carry the entry's content. A
/// version that matches no entry, or more than one, or an entry two versions
/// match, refuses the whole answer. No specification governs the matching:
/// our own design.
fn pair(
    contribution: &ContributionUid,
    mapped: &[Mapped<'_>],
    stored: Vec<(ObjectVersionId, Composition)>,
) -> Result<Vec<ObjectVersionId>, Refused> {
    let keys: Vec<AuditKey> = mapped
        .iter()
        .map(|entry| AuditKey::of(&entry.composition))
        .collect();
    let mut placed: Vec<Option<ObjectVersionId>> = vec![None; mapped.len()];
    for (version, composition) in stored {
        let key = AuditKey::of(&composition);
        let sharing = keys.iter().filter(|candidate| **candidate == key).count();
        let matching: Vec<usize> = mapped
            .iter()
            .zip(&keys)
            .enumerate()
            .filter(|(_, (entry, candidate))| {
                **candidate == key
                    && (sharing == 1 || same_content(&entry.composition, &composition))
            })
            .map(|(index, _)| index)
            .collect();
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
        *slot = Some(version);
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
/// list whose length is not the entries' cannot bind the entries, and the
/// refusal names the contribution so it can be reconciled. The list states no
/// order, so [`pair`] matches each version to its entry.
fn listed_versions(
    contribution: &ContributionUid,
    stored: &Contribution,
    expected: usize,
) -> Result<Vec<ObjectVersionId>, Refused> {
    let unreadable = |why: String| unbound(contribution, &why);
    let references: &[ObjectRef] = stored.versions.as_ref();
    if references.len() != expected {
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

/// Returns the refusal a documented CDR refusal decides through `row`.
fn upstream_refusal(
    row: status::Row,
    upstream: &ferrobridge_openehr::error::UpstreamError,
) -> Refused {
    Refused::of_answer(&status::Answer::new(row, status::diagnostics(upstream)))
}

/// Returns the refusal a rejected contribution renders as.
///
/// Nothing was committed, so the answer names every entry the Bundle mapped:
/// a caller cannot tell from a partial list which entries still stand.
fn refuse_all(
    mapped: &[Mapped<'_>],
    row: &status::Row,
    upstream: &ferrobridge_openehr::error::UpstreamError,
) -> Refused {
    let detail = status::diagnostics(upstream);
    let mut issues = vec![
        Issue::error(row.issue())
            .diagnosing(detail.clone())
            .detailing("the CDR refused the contribution, so no entry of this Bundle is stored"),
    ];
    for entry in mapped {
        issues.push(
            Issue::error(row.issue())
                .diagnosing(detail.clone())
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
