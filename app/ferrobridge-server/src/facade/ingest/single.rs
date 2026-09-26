// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The single create and update: what the identity map knows of one
//! resource, the first version of its composition, and a later one.

use http::StatusCode;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_its::rest::generated::ehr::client::CompositionCreateOutcome;
use openehr_its::rest::generated::ehr::client::CompositionUpdateOutcome;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::Prefer;
use crate::cdr::ids::EhrId;
use crate::facade::commit;
use crate::facade::ehr;
use crate::facade::identity::claims::Claim;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::chain;
use crate::facade::programs::Loaded;
use crate::facade::request::Inbound;
use crate::facade::status;

use super::Ingest;
use super::Provenance;
use super::Recognised;
use super::Refused;
use super::Written;
use super::binding::representation;
use super::binding::resource_source;
use super::binding::strict_read;
use super::claim::unclaimed;
use super::engine_refusal;
use super::pairing::same_content;
use super::reconcile::one_composition;
use super::refusal::cdr_refusal;
use super::refusal::ehr_refusal;
use super::refusal::upstream_refusal;
use super::store_refusal;

impl Ingest<'_> {
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
    ///
    /// [`SourceVersion`]: crate::facade::identity::record::SourceVersion
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
    ///
    /// [`Delivery::Reconciled`]: super::Delivery::Reconciled
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
}
