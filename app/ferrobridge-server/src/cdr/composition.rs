// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The composition operations: create, update, read and delete.

use http::HeaderMap;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::rest::generated::ehr::CompositionCreateParams;
use openehr_its::rest::generated::ehr::CompositionDeleteParams;
use openehr_its::rest::generated::ehr::CompositionGetParams;
use openehr_its::rest::generated::ehr::CompositionUpdateParams;
use openehr_its::rest::generated::ehr::client::CompositionCreateOutcome;
use openehr_its::rest::generated::ehr::client::CompositionDeleteOutcome;
use openehr_its::rest::generated::ehr::client::CompositionGetOutcome;
use openehr_its::rest::generated::ehr::client::CompositionUpdateOutcome;
use openehr_its::rest::generated::ehr::client::EhrClient;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::cdr::commit::CommitContext;
use crate::cdr::error::CdrError;
use crate::cdr::ids::EhrId;

use super::Answered;
use super::CdrClient;
use super::Prefer;
use super::VersionAtTime;
use super::answered_by;
use super::if_match_value;
use super::representation;

impl CdrClient {
    /// Commits the first version of a composition into the EHR `ehr_id`.
    ///
    /// The body is canonical JSON, the mandatory composition representation of
    /// ITS-REST 1.1.0, and `commit` renders the committal metadata headers.
    /// The call is a `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the commit metadata cannot travel in a header
    /// or the call did not reach a documented answer.
    pub async fn create_composition(
        &self,
        ehr_id: &EhrId,
        composition: &Composition,
        commit: &CommitContext,
        prefer: Prefer,
    ) -> Result<Answered<CompositionCreateOutcome>, CdrError> {
        let headers = commit.headers()?;
        let client = self.call(HeaderMap::new())?;
        let params = CompositionCreateParams {
            ehr_id: String::from(ehr_id.as_str()),
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_item_tag: None,
            openehr_version_item_tag: None,
            openehr_version: headers.version,
            openehr_audit_details: headers.audit_details,
            openehr_template_id: headers.template_id,
        };
        let answered = EhrClient::new(&client)
            .composition_create(&params, composition)
            .await;
        answered_by(&client, answered)
    }

    /// Commits a new version of the composition `versioned_object_uid`.
    ///
    /// `preceding` is the latest version the caller knows of; it travels as
    /// the `If-Match` header, quoted and without the `W/` an `ETag` carries.
    /// The call is idempotent under that precondition, so the retry budget
    /// applies.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the commit metadata cannot travel in a header
    /// or the call did not reach a documented answer.
    pub async fn update_composition(
        &self,
        ehr_id: &EhrId,
        versioned_object_uid: &HierObjectId,
        preceding: &ObjectVersionId,
        composition: &Composition,
        commit: &CommitContext,
        prefer: Prefer,
    ) -> Result<Answered<CompositionUpdateOutcome>, CdrError> {
        let headers = commit.headers()?;
        let client = self.call(HeaderMap::new())?;
        let params = CompositionUpdateParams {
            ehr_id: String::from(ehr_id.as_str()),
            uid_based_id: String::from(versioned_object_uid.value()),
            if_match: if_match_value(preceding),
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_item_tag: None,
            openehr_version_item_tag: None,
            openehr_version: headers.version,
            openehr_audit_details: headers.audit_details,
            openehr_template_id: headers.template_id,
        };
        let answered = EhrClient::new(&client)
            .composition_update(&params, composition)
            .await;
        answered_by(&client, answered)
    }

    /// Retrieves a version of a composition.
    ///
    /// "The `uid_based_id` can take a form of an `OBJECT_VERSION_ID` identifier
    /// taken from `VERSION.uid.value` …, or a form of a `HIER_OBJECT_ID`
    /// identifier taken from `VERSIONED_OBJECT.uid.value`"
    /// (`ehr-codegen.openapi.yaml`, `composition_get`). `at` selects the
    /// version extant at that time and is only meaningful when `uid_based_id`
    /// is a version container.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn composition(
        &self,
        ehr_id: &EhrId,
        uid_based_id: &UidBasedId,
        at: Option<&VersionAtTime>,
    ) -> Result<Answered<CompositionGetOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = CompositionGetParams {
            ehr_id: String::from(ehr_id.as_str()),
            uid_based_id: String::from(uid_based_id.value()),
            version_at_time: at.map(|at| String::from(at.as_str())),
            accept: None,
        };
        let answered = EhrClient::new(&client).composition_get(&params).await;
        answered_by(&client, answered)
    }

    /// Deletes the composition whose latest version is `preceding`.
    ///
    /// "The `uid_based_id` MUST be in a form of an `OBJECT_VERSION_ID`
    /// identifier taken from the last (most recent) `VERSION.uid.value`"
    /// (`ehr-codegen.openapi.yaml`, `composition_delete`), which is why this
    /// call takes a version identifier and is retryable.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn delete_composition(
        &self,
        ehr_id: &EhrId,
        preceding: &ObjectVersionId,
    ) -> Result<Answered<CompositionDeleteOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = CompositionDeleteParams {
            ehr_id: String::from(ehr_id.as_str()),
            uid_based_id: String::from(preceding.value()),
            openehr_version: None,
            openehr_audit_details: None,
        };
        let answered = EhrClient::new(&client).composition_delete(&params).await;
        answered_by(&client, answered)
    }
}
