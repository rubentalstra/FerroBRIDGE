// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Bundle path: every mapped entry committed as one CONTRIBUTION.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use fhir_types::codec::Value;
use http::StatusCode;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::generated::ehr::Versionable;
use openehr_its::rest::generated::ehr::client::ContributionCreateOutcome;
use openehr_rm::v1_2::common::change_control::contribution::Contribution;

use crate::cdr::Prefer;
use crate::cdr::Returned;
use crate::cdr::error::CdrError;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::EhrId;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::record::SourceVersion;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::status;

use super::Commit;
use super::Committed;
use super::EntryOutcome;
use super::Ingest;
use super::Ingested;
use super::Mapped;
use super::Partition;
use super::Pending;
use super::Placed;
use super::Provenance;
use super::Recorded;
use super::Refused;
use super::Scope;
use super::UnmappedEntries;
use super::binding::resource_source;
use super::claim::claim_keys;
use super::claim::contended_entries;
use super::entries::nothing_mapped;
use super::reconcile::one_contribution;
use super::reconcile::versions_of;
use super::refusal::cdr_refusal;
use super::refusal::ehr_refusal;
use super::refusal::refuse_all;
use super::refusal::stored_identifier;

/// Returns the change a version that follows `preceding` makes: a creation
/// when it follows nothing, a modification otherwise.
const fn change_of(preceding: Option<&ObjectVersionId>) -> commit::Change {
    match preceding {
        None => commit::Change::Creation,
        Some(_) => commit::Change::Modification,
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

/// Refuses a Bundle in which two entries carry one resource.
///
/// "A resource can only appear in a transaction once (by identity)"
/// (<https://hl7.org/fhir/R4/http.html#transaction>), and the identity is the
/// resource type and the `id`, whatever `meta.versionId` each entry states.
/// One `id` at two versions is `400 invalid` naming both entries, since
/// neither is the version the other follows and an unknown `id` would commit
/// two compositions. An entry that repeats another's version as well keeps
/// the `422 duplicate` that names the repeat (no specification governs the
/// split: our own design).
fn distinct(mapped: &[Mapped<'_>], sources: &[Option<SourceVersion>]) -> Result<(), Refused> {
    let mut seen: BTreeMap<String, (&str, String)> = BTreeMap::new();
    let mut repeated = Vec::new();
    let mut versioned = Vec::new();
    for (entry, source) in mapped.iter().zip(sources) {
        let Some(source) = source.as_ref() else {
            continue;
        };
        let known = seen.get(&source.once_key()).cloned();
        let Some((first_url, first_key)) = known else {
            seen.insert(
                source.once_key(),
                (entry.full_url.as_str(), source.storage_key()),
            );
            continue;
        };
        if first_key == source.storage_key() {
            repeated.push(
                Issue::error(IssueType::Duplicate)
                    .diagnosing(format!(
                        "{}/{} appears in this transaction more than once",
                        source.resource_type(),
                        source.id()
                    ))
                    .at(entry.full_url.clone()),
            );
        } else {
            versioned.push(
                Issue::error(IssueType::Invalid)
                    .diagnosing(format!(
                        "{}/{} appears in this transaction at two versions, in {first_url} and {}; a transaction carries one resource once",
                        source.resource_type(),
                        source.id(),
                        entry.full_url
                    ))
                    .at(String::from(first_url))
                    .at(entry.full_url.clone()),
            );
        }
    }
    if !versioned.is_empty() {
        versioned.extend(repeated);
        return Err(Refused::of(StatusCode::BAD_REQUEST, versioned));
    }
    if repeated.is_empty() {
        Ok(())
    } else {
        Err(Refused::of(StatusCode::UNPROCESSABLE_ENTITY, repeated))
    }
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
pub(super) fn outcomes(partition: Partition<'_>, placed: Vec<Placed>) -> Vec<EntryOutcome> {
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

impl Ingest<'_> {
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
    ///
    /// [`CommittedSource`]: crate::facade::identity::record::CommittedSource
    /// [`Claims`]: crate::facade::identity::claims::Claims
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
}
