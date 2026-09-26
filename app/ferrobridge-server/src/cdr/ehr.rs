// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR operations: create, read by id, and read by subject.

use http::HeaderMap;
use openehr_its::rest::generated::ehr::EhrCreateParams;
use openehr_its::rest::generated::ehr::EhrGetByIdParams;
use openehr_its::rest::generated::ehr::EhrGetBySubjectParams;
use openehr_its::rest::generated::ehr::client::EhrClient;
use openehr_its::rest::generated::ehr::client::EhrCreateOutcome;
use openehr_its::rest::generated::ehr::client::EhrGetByIdOutcome;
use openehr_its::rest::generated::ehr::client::EhrGetBySubjectOutcome;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;

use crate::cdr::error::CdrError;
use crate::cdr::ids::EhrId;
use crate::cdr::ids::SubjectId;
use crate::cdr::ids::SubjectNamespace;

use super::Answered;
use super::CdrClient;
use super::Prefer;
use super::answered_by;
use super::representation;

impl CdrClient {
    /// Creates an EHR with a server-assigned identifier.
    ///
    /// `status` is the optional `EHR_STATUS` the service commits with the new
    /// EHR (`ehr-codegen.openapi.yaml`, `ehr_create`). The call is a `POST`,
    /// so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn create_ehr(
        &self,
        status: Option<&EhrStatus>,
        prefer: Prefer,
    ) -> Result<Answered<EhrCreateOutcome>, CdrError> {
        let client = self.call(HeaderMap::new())?;
        let params = EhrCreateParams {
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_version: None,
            openehr_audit_details: None,
        };
        let answered = EhrClient::new(&client).ehr_create(&params, status).await;
        answered_by(&client, answered)
    }

    /// Retrieves the EHR with `ehr_id` (`ehr-codegen.openapi.yaml`,
    /// `ehr_get_by_id`).
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn ehr(&self, ehr_id: &EhrId) -> Result<Answered<EhrGetByIdOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = EhrGetByIdParams {
            ehr_id: String::from(ehr_id.as_str()),
            accept: None,
        };
        let answered = EhrClient::new(&client).ehr_get_by_id(&params).await;
        answered_by(&client, answered)
    }

    /// Retrieves the EHR whose subject is `subject_id` in `namespace`.
    ///
    /// The parameters are matched against
    /// `EHR_STATUS.subject.external_ref.id.value` and
    /// `EHR_STATUS.subject.external_ref.namespace`
    /// (`ehr-codegen.openapi.yaml`, `ehr_get_by_subject`).
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn ehr_by_subject(
        &self,
        subject_id: &SubjectId,
        namespace: &SubjectNamespace,
    ) -> Result<Answered<EhrGetBySubjectOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = EhrGetBySubjectParams {
            subject_id: String::from(subject_id.as_str()),
            subject_namespace: String::from(namespace.as_str()),
            accept: None,
        };
        let answered = EhrClient::new(&client).ehr_get_by_subject(&params).await;
        answered_by(&client, answered)
    }
}
