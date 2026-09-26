// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reconcile paths: a delivery an earlier one committed or consumed,
//! bound or answered from what the CDR holds, committing nothing.

use http::StatusCode;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::rest::generated::ehr::client::CompositionGetOutcome;
use openehr_its::rest::generated::ehr::client::ContributionGetOutcome;
use openehr_rm::v1_2::common::change_control::contribution::Contribution;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::Returned;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::EhrId;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::record::CommittedSource;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::record::SourceVersion;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::chain;
use crate::facade::programs::Loaded;
use crate::facade::request::Inbound;
use crate::facade::status;

use super::Commit;
use super::Delivery;
use super::Ingest;
use super::Ingested;
use super::Mapped;
use super::Partition;
use super::Placed;
use super::Provenance;
use super::Recorded;
use super::Refused;
use super::Scope;
use super::Standing;
use super::Written;
use super::binding::representation;
use super::binding::resource_source;
use super::binding::strict_read;
use super::engine_refusal;
use super::pairing::pair;
use super::refusal::cdr_refusal;
use super::refusal::stored_identifier;
use super::refusal::unbound;
use super::refusal::upstream_refusal;
use super::store_refusal;
use super::transaction::outcomes;

/// Returns the EHR and the contribution a commit record names.
fn committed_ids(committed: &CommittedSource) -> Result<(EhrId, ContributionUid), Refused> {
    let ehr_id = EhrId::new(&committed.ehr_id).map_err(|error| stored_identifier(&error))?;
    let contribution = ContributionUid::new(&committed.contribution_uid)
        .map_err(|error| stored_identifier(&error))?;
    Ok((ehr_id, contribution))
}

/// Returns the one composition every consumed version of `source`'s `id`
/// names, or nothing when the map consumed none.
///
/// Two versions of one `id` that name two compositions leave no one
/// composition a later version revises, so the create is refused with the
/// record intact (no specification governs this: our own design).
pub(super) fn one_composition(
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

/// Returns the contribution every entry of a Bundle was committed in, when
/// one contribution committed them all.
pub(super) fn one_contribution(committed: &[Option<CommittedSource>]) -> Option<&CommittedSource> {
    let first = committed.first()?.as_ref()?;
    committed
        .iter()
        .all(|record| record.as_ref() == Some(first))
        .then_some(first)
}

/// Returns the versions of `paired`, in order.
pub(super) fn versions_of(paired: Vec<(ObjectVersionId, Composition)>) -> Vec<ObjectVersionId> {
    paired.into_iter().map(|(version, _)| version).collect()
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

impl Ingest<'_> {
    /// Binds `inbound` from the contribution a transaction recorded as
    /// committing it, when the identity map holds that record and no
    /// consumed source.
    ///
    /// The contribution is read back and the version whose composition the
    /// resource maps to is found the way a re-sent transaction finds it
    /// ([`pair`]), so the single and the transaction paths share one rule.
    pub(super) async fn reconcile_resource(
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

    /// Binds a Bundle whose every entry an earlier delivery committed in
    /// `committed` and did not finish binding, from the contribution read
    /// back.
    ///
    /// Nothing is committed a second time: the versions come from the
    /// CONTRIBUTION the CDR holds, verified entry by entry as a first delivery
    /// verifies them.
    pub(super) async fn reconcile(
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

    /// Answers a Bundle whose every entry the identity map already consumed,
    /// with each entry where its first delivery put it.
    pub(super) async fn replay(
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

    /// Returns what the identity map records of one Bundle entry's `source`:
    /// the exact source consumed, the contribution that committed it, and,
    /// when neither, the composition another version of its `id` produced.
    ///
    /// Only an entry mapped from a resource has versions of one `id`; the
    /// entries of one message share its control id and are no versions of
    /// each other. The by-id lookup is the one a single create runs
    /// ([`Ingest::recognise`]).
    pub(super) fn recorded(
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

    /// Records that every keyed entry of a Bundle was committed in
    /// `contribution`, before any of them is bound.
    pub(super) fn record_commitment(
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

    /// Returns the version each mapped entry produced in `contribution`, with
    /// the composition the CDR holds for it, in entry order.
    ///
    /// The versions come from the representation the commit answered, or,
    /// when it answered none, from the CONTRIBUTION read back by its uid
    /// (`ehr-codegen.openapi.yaml`, `contribution_get`). Each version is then
    /// read and matched to its entry ([`pair`]) under `scope`.
    pub(super) async fn committed_versions(
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
    pub(super) async fn standing(&self, known: &ConsumedSource) -> Result<Standing, Refused> {
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
    pub(super) async fn read(
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
