// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Resolving the EHR one composition is written into.
//!
//! The identity map is asked first, then the CDR by subject id and namespace
//! (`ehr-codegen.openapi.yaml`, `ehr_get_by_subject`), and only then does the
//! configured policy decide whether an EHR is created
//! (`docs/architecture.md` §4.6). The `EHR_STATUS.subject` shape is a
//! `PARTY_SELF` over a `PARTY_REF` whose `GENERIC_ID` scheme is the namespace,
//! which is what makes the same lookup find it again (§12).

use ferrobridge_openehr::client::Client;
use ferrobridge_openehr::ehr::CreateEhrOutcome;
use ferrobridge_openehr::ehr::EhrOutcome;
use ferrobridge_openehr::ids::EhrId;
use ferrobridge_openehr::ids::SubjectId;
use ferrobridge_openehr::ids::SubjectNamespace;
use ferrobridge_openehr::prefer::Prefer;
use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::v1_2::common::generic::party_self::PartySelf;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;
use openehr_rm::v1_2::data_types::text::dv_text::DvTextData;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;

use crate::facade::identity::PersonId;
use crate::facade::identity::store::Store;
use crate::facade::identity::store::StoreError;

/// The archetype every openEHR `EHR_STATUS` declares.
///
/// The root archetype of the class is `openEHR-EHR-EHR_STATUS.generic.v1`
/// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/ehr.html>).
const EHR_STATUS_ARCHETYPE: &str = "openEHR-EHR-EHR_STATUS.generic.v1";

/// The reference-model class a subject reference points at.
const PARTY_TYPE: &str = "PERSON";

/// Whether the facade may create an EHR it does not find.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// Only an EHR the CDR already holds is written into.
    #[default]
    Existing,
    /// An unknown subject gets an EHR on its first write.
    CreateOnFirstWrite,
}

/// Why no EHR could be resolved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EhrError {
    /// The subject identifier cannot travel in the CDR lookup.
    #[error("the subject {subject} cannot be looked up in the CDR")]
    Subject {
        /// The subject that was to be looked up.
        subject: String,
        /// What the identifier's constructor reported.
        #[source]
        source: Box<ferrobridge_openehr::ids::IdError>,
    },
    /// The CDR call did not reach a documented answer.
    #[error("the EHR lookup did not reach a documented answer")]
    Client {
        /// What the client reported.
        #[source]
        source: Box<ferrobridge_openehr::error::Error>,
    },
    /// The CDR refused the EHR creation.
    #[error("the CDR refused to create an EHR for {subject}: {detail}")]
    Refused {
        /// The subject the EHR was for.
        subject: String,
        /// What the CDR answered.
        detail: String,
    },
    /// The CDR holds no EHR and the policy does not create one.
    #[error(
        "the CDR holds no EHR for {subject}, and the facade is configured to write into an existing EHR only"
    )]
    Absent {
        /// The subject with no EHR.
        subject: String,
    },
    /// The identity store refused.
    #[error("the identity store could not resolve the EHR")]
    Store {
        /// What the store reported.
        #[source]
        source: Box<StoreError>,
    },
}

/// Returns the EHR of `person`, resolving or creating it per `policy`.
///
/// # Errors
///
/// Returns [`EhrError::Absent`] when no EHR exists and `policy` is
/// [`Policy::Existing`], [`EhrError::Refused`] when the CDR refuses the
/// creation, and the transport and store refusals of [`EhrError`].
pub async fn resolve(
    client: &Client,
    store: &dyn Store,
    person: &PersonId,
    policy: Policy,
) -> Result<EhrId, EhrError> {
    if let Some(known) = store.ehr_of(person).map_err(store_error)? {
        return Ok(known);
    }
    let subject_id = SubjectId::new(person.id()).map_err(|source| EhrError::Subject {
        subject: person.to_string(),
        source: Box::new(source),
    })?;
    let namespace =
        SubjectNamespace::new(person.namespace()).map_err(|source| EhrError::Subject {
            subject: person.to_string(),
            source: Box::new(source),
        })?;
    let found = client
        .ehr_by_subject(&subject_id, &namespace)
        .await
        .map_err(client_error)?;
    let ehr_id = match found {
        EhrOutcome::Found(ehr) => ehr_id_of(&ehr)?,
        EhrOutcome::NotFound(..) => match policy {
            Policy::Existing => {
                return Err(EhrError::Absent {
                    subject: person.to_string(),
                });
            }
            Policy::CreateOnFirstWrite => create(client, person).await?,
        },
        other => {
            return Err(EhrError::Refused {
                subject: person.to_string(),
                detail: format!(
                    "the client answered an outcome this version does not read ({other:?})"
                ),
            });
        }
    };
    store.record_ehr(person, &ehr_id).map_err(store_error)
}

/// Creates an EHR whose `EHR_STATUS.subject` names `person`.
async fn create(client: &Client, person: &PersonId) -> Result<EhrId, EhrError> {
    let status = status_of(person);
    match client
        .create_ehr(Some(&status), Prefer::Minimal)
        .await
        .map_err(client_error)?
    {
        CreateEhrOutcome::Created { ehr_id, .. } => Ok(ehr_id),
        CreateEhrOutcome::BadRequest(upstream) | CreateEhrOutcome::Conflict(upstream) => {
            Err(EhrError::Refused {
                subject: person.to_string(),
                detail: crate::facade::status::diagnostics(&upstream),
            })
        }
        other => Err(EhrError::Refused {
            subject: person.to_string(),
            detail: format!(
                "the client answered an outcome this version does not read ({other:?})"
            ),
        }),
    }
}

/// Returns the `EHR_STATUS` an EHR for `person` is created with.
#[must_use]
pub fn status_of(person: &PersonId) -> EhrStatus {
    EhrStatus {
        name: DvText::DvText(DvTextData {
            value: String::from("EHR Status"),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        }),
        archetype_node_id: String::from(EHR_STATUS_ARCHETYPE),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        subject: PartySelf {
            external_ref: Some(PartyRef {
                namespace: String::from(person.namespace()),
                r#type: String::from(PARTY_TYPE),
                id: ObjectId::GenericId(GenericId {
                    value: String::from(person.id()),
                    scheme: String::from(person.namespace()),
                }),
            }),
        },
        is_queryable: true,
        is_modifiable: true,
        other_details: None,
    }
}

/// Returns the `ehr_id` of a fetched EHR.
fn ehr_id_of(ehr: &openehr_rm::v1_2::ehr::ehr::Ehr) -> Result<EhrId, EhrError> {
    EhrId::new(ehr.ehr_id.value()).map_err(|source| EhrError::Subject {
        subject: ehr.ehr_id.value().to_owned(),
        source: Box::new(source),
    })
}

/// Wraps a client refusal as an EHR refusal that carries it.
fn client_error(source: ferrobridge_openehr::error::Error) -> EhrError {
    EhrError::Client {
        source: Box::new(source),
    }
}

/// Wraps a store refusal as an EHR refusal that carries it.
fn store_error(source: StoreError) -> EhrError {
    EhrError::Store {
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::{EHR_STATUS_ARCHETYPE, PARTY_TYPE, Policy, status_of};
    use crate::facade::identity::PersonId;
    use openehr_base::v1_3::base_types::identification::object_id::ObjectId;

    #[test]
    fn the_default_policy_writes_into_an_existing_ehr_only() {
        assert_eq!(Policy::Existing, Policy::default());
    }

    #[test]
    fn the_status_subject_is_a_generic_id_in_the_namespace() {
        let person = PersonId::new("http://example.org/mrn", "p-1").expect("a legal person id");
        let status = status_of(&person);
        assert_eq!(EHR_STATUS_ARCHETYPE, status.archetype_node_id);
        let reference = status
            .subject
            .external_ref
            .expect("the subject names an external reference");
        assert_eq!("http://example.org/mrn", reference.namespace);
        assert_eq!(PARTY_TYPE, reference.r#type);
        let ObjectId::GenericId(id) = reference.id else {
            panic!("the subject id is a GENERIC_ID whose scheme is the namespace");
        };
        assert_eq!("p-1", id.value);
        assert_eq!("http://example.org/mrn", id.scheme);
    }

    #[test]
    fn a_created_ehr_is_queryable_and_modifiable() {
        let person = PersonId::new("http://example.org/mrn", "p-1").expect("a legal person id");
        let status = status_of(&person);
        assert!(status.is_queryable);
        assert!(status.is_modifiable);
    }
}
