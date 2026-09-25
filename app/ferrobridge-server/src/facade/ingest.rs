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

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::composition::CreateCompositionOutcome;
use ferrobridge_openehr::composition::UpdateCompositionOutcome;
use ferrobridge_openehr::contribution::CreateContributionOutcome;
use ferrobridge_openehr::ids::ContributionUid;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::ids::ObjectVersionId;
use ferrobridge_openehr::ids::VersionedObjectUid;
use ferrobridge_openehr::prefer::Prefer;
use ferrobridge_openehr::prefer::Returned;
use fhir_types::codec::Value;
use fhirconnect::engine::origin::SourceItem;
use fhirconnect::resolve::program::TemplateId;
use fhirconnect::resolve::select::SelectError;
use http::StatusCode;
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::generated::ehr::Versionable;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::facade::Settings;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::engine;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::PersonId;
use crate::facade::identity::derive;
use crate::facade::identity::derive::EntryKey;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// The `fullUrl` of the entry, or its position when it has none.
    pub full_url: String,
    /// The `resourceType` the entry declares.
    pub resource_type: String,
    /// The template of the composition the entry produced.
    pub template_id: String,
}

/// What happened to one Bundle entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryOutcome {
    /// The entry mapped and its composition is stored.
    Committed(Committed),
    /// No program ran over the entry.
    Skipped(Skipped),
}

/// What one committed Bundle produced.
///
/// Every committed entry went into the CDR as one version of the one
/// CONTRIBUTION this names (`ehr-codegen.openapi.yaml`, `contribution_create`).
#[derive(Debug, Clone)]
pub struct Ingested {
    /// The EHR every composition was written into.
    ehr_id: EhrId,
    /// The contribution that carries every committed composition.
    contribution: ContributionUid,
    /// One outcome per entry, in Bundle order.
    entries: Vec<EntryOutcome>,
}

impl Ingested {
    /// Returns the EHR every composition was written into.
    #[must_use]
    pub const fn ehr_id(&self) -> &EhrId {
        &self.ehr_id
    }

    /// Returns the contribution that carries every committed composition.
    #[must_use]
    pub const fn contribution(&self) -> &ContributionUid {
        &self.contribution
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
struct Mapped {
    /// The `fullUrl` the Bundle gave the entry, for a refusal that names it.
    full_url: String,
    /// The composition the entry produced.
    composition: Box<Composition>,
}

/// What mapping one Bundle entry produced.
#[derive(Debug)]
enum Mapping {
    /// The entry mapped, for the subject it names.
    Mapped(Mapped, Committed, PersonId),
    /// No program ran: the skip, and the issue that refuses the Bundle.
    Unmapped(Skipped, Issue),
}

/// Every entry of a Bundle, mapped and partitioned.
#[derive(Debug, Default)]
struct Partition {
    /// The entries that mapped, in Bundle order.
    mapped: Vec<Mapped>,
    /// One outcome per entry that mapped or was skipped, in Bundle order.
    outcomes: Vec<EntryOutcome>,
    /// The issues that refuse the Bundle.
    failures: Vec<Issue>,
    /// The one subject every mapped entry names.
    subject: Option<PersonId>,
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
        container: &VersionedObjectUid,
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
                    answer = answer.with_entity_tag(String::from(latest.version_tree_id()));
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
    /// # Errors
    ///
    /// Returns a `422` [`Refused`] naming every entry that does not map (and,
    /// under [`UnmappedEntries::Refuse`], every entry no program maps), a
    /// second subject, a Bundle with nothing to commit, and an EHR that cannot
    /// be resolved; the CDR's refusal of the contribution through the status
    /// table, naming every mapped entry.
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
        let Some(subject) = partition.subject else {
            return Err(Refused::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                Issue::error(IssueType::Required)
                    .diagnosing("the Bundle carries no entry this server maps"),
            ));
        };
        let ehr_id = ehr::resolve(&self.client, self.store, &subject, self.settings.ehr_policy)
            .await
            .map_err(|error| {
                Refused::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Issue::error(IssueType::Processing).diagnosing(chain(&error)),
                )
            })?;
        let contribution = self.commit_all(&ehr_id, &partition.mapped).await?;
        Ok(Ingested {
            ehr_id,
            contribution,
            entries: partition.outcomes,
        })
    }

    /// Maps every entry, collecting the refusals rather than stopping at the
    /// first.
    fn map_entries(
        &self,
        entries: &[Value],
        unmapped: UnmappedEntries,
        provenance: &Provenance,
    ) -> Partition {
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
            match self.map_one(resource, &full_url, provenance) {
                Ok(Mapping::Mapped(one, committed, person)) => {
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
                    partition.mapped.push(one);
                    partition.outcomes.push(EntryOutcome::Committed(committed));
                }
                Ok(Mapping::Unmapped(skipped, issue)) => match unmapped {
                    UnmappedEntries::Refuse => partition.failures.push(issue),
                    UnmappedEntries::SkipAndCount => {
                        partition.outcomes.push(EntryOutcome::Skipped(skipped));
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
        provenance: &Provenance,
    ) -> Result<Mapping, Issue> {
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
            Mapped {
                full_url: String::from(full_url),
                composition: Box::new(rm),
            },
            Committed {
                full_url: String::from(full_url),
                resource_type: String::from(inbound.resource_type()),
                template_id: String::from(composition.template_id()),
            },
            subject,
        ))
    }

    /// Commits every mapped entry as one CONTRIBUTION.
    async fn commit_all(
        &self,
        ehr_id: &EhrId,
        mapped: &[Mapped],
    ) -> Result<ContributionUid, Refused> {
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
        let answered = self
            .client
            .create_contribution(ehr_id, &contribution, Prefer::Minimal)
            .await
            .map_err(|error| Refused::of_answer(&status::of_client_error(&error)))?;
        match answered {
            CreateContributionOutcome::Created {
                contribution_uid, ..
            } => Ok(contribution_uid),
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
        let key = EntryKey::new(version.versioned_object_uid(), entry.path(), 0);
        let map_key = derive::map_key(&key);
        let resource_type = inbound.resource_type();
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
            versioned_object_uid: String::from(version.versioned_object_uid().as_str()),
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
        if let Some(external) = inbound.id() {
            let source = SourceVersion::new(
                resource_type,
                external.clone(),
                inbound.version_id().map(str::to_owned),
            );
            self.store
                .record_consumed(
                    &source,
                    &ConsumedSource {
                        ehr_id: stood.ehr_id.clone(),
                        versioned_object_uid: stood.versioned_object_uid.clone(),
                        internal_id: String::from(id.as_str()),
                        context: stood.context.clone(),
                    },
                )
                .map_err(|error| store_refusal(&error))?;
        }
        Ok(Written {
            id,
            ehr_id: ehr_id.clone(),
            version: version.clone(),
            composition: composition.clone(),
        })
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
    mapped: &[Mapped],
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
