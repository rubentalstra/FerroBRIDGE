// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Reading one inbound FHIR resource: the codec, the profiles, the subject.
//!
//! Every body is read through the `fhir-types` strict codec, so an unknown
//! property is a refusal rather than a silently dropped element
//! (`docs/architecture.md` §4.1). What the facade then needs off the document
//! is its `resourceType`, its `id` and `meta.versionId`, the set of
//! `meta.profile`, and the subject the composition is written into.

use fhir_types::codec::Value;

use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::PersonId;

/// Why one inbound document is not a resource this facade can take.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RequestError {
    /// The body is not JSON.
    #[error("the request body is not JSON: {reason}")]
    NotJson {
        /// What the reader reported.
        reason: String,
    },
    /// The body is JSON but not a FHIR resource object.
    #[error("the request body is no FHIR resource: it is not a JSON object")]
    NotAnObject,
    /// The body declares no `resourceType`, or one that is not a string.
    #[error("the request body declares no resourceType")]
    NoResourceType,
    /// The body is not the resource its `resourceType` declares.
    ///
    /// The strict codec refuses an unknown property rather than dropping it
    /// (`docs/architecture.md` §4.1).
    #[error("{location}: {reason}")]
    Structure {
        /// The element path the codec refused at.
        location: String,
        /// Why it refused.
        reason: String,
    },
    /// The body declares a `resourceType` the request path does not name.
    #[error("the request body declares {found}, and the path names {wanted}")]
    WrongResourceType {
        /// The type the body declares.
        found: String,
        /// The type the path names.
        wanted: String,
    },
    /// The body carries no `id` where one is required.
    #[error("the request body carries no id")]
    NoId,
    /// The `id` the body carries is not usable as a key.
    #[error("the id the request body carries cannot be a key")]
    Id {
        /// What the identifier's constructor reported.
        #[source]
        source: crate::facade::identity::IdError,
    },
    /// The resource names no subject, so no EHR can hold it.
    #[error(
        "the resource names no subject; a clinical resource is written into the EHR of its subject"
    )]
    NoSubject,
    /// The subject the resource names cannot be a person key.
    #[error("the subject the resource names cannot be a person key")]
    Subject {
        /// What the identifier's constructor reported.
        #[source]
        source: crate::facade::identity::IdError,
    },
}

/// One inbound resource, read.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// The document, as the strict codec read it.
    document: Value,
    /// The `resourceType` the document declares.
    resource_type: String,
    /// The `id` the sender wrote, when it wrote one.
    id: Option<ExternalResourceId>,
    /// The `meta.versionId` the sender wrote, when it wrote one.
    version_id: Option<String>,
    /// The canonical URLs of `meta.profile`, in document order.
    profiles: Vec<String>,
}

impl Inbound {
    /// Reads `body` as a resource of `wanted`.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::NotJson`] for a body that is not JSON,
    /// [`RequestError::NotAnObject`] for JSON that is not an object,
    /// [`RequestError::NoResourceType`] for a document declaring none, and
    /// [`RequestError::WrongResourceType`] when it declares another type than
    /// the request path names.
    pub fn read(body: &[u8], wanted: &str) -> Result<Self, RequestError> {
        let text = core::str::from_utf8(body).map_err(|source| RequestError::NotJson {
            reason: source.to_string(),
        })?;
        let document =
            serde_json::from_str::<Value>(text).map_err(|source| RequestError::NotJson {
                reason: source.to_string(),
            })?;
        Self::of(document, wanted)
    }

    /// Reads one already-parsed document as a resource of `wanted`.
    ///
    /// `wanted` is empty when the caller takes whatever type the document
    /// declares, which is the transaction path.
    ///
    /// # Errors
    ///
    /// The errors of [`Inbound::read`] beyond the JSON reader's.
    pub fn of(document: Value, wanted: &str) -> Result<Self, RequestError> {
        let object = document
            .as_object()
            .ok_or(RequestError::NotAnObject)?
            .clone();
        let resource_type = object
            .get("resourceType")
            .and_then(Value::as_str)
            .ok_or(RequestError::NoResourceType)?
            .to_owned();
        if !wanted.is_empty() && resource_type != wanted {
            return Err(RequestError::WrongResourceType {
                found: resource_type,
                wanted: wanted.to_owned(),
            });
        }
        let mut path = fhir_types::codec::Path::root(&resource_type);
        let _checked: fhir_types::r4::resource::Resource =
            fhir_types::codec::Json::from_json(&object, &mut path).map_err(|refusal| {
                RequestError::Structure {
                    location: refusal.path,
                    reason: refusal.kind.to_string(),
                }
            })?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .map(ExternalResourceId::new)
            .transpose()
            .map_err(|source| RequestError::Id { source })?;
        let meta = object.get("meta");
        let version_id = meta
            .and_then(|meta| meta.get("versionId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let profiles = meta
            .and_then(|meta| meta.get("profile"))
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            document,
            resource_type,
            id,
            version_id,
            profiles,
        })
    }

    /// Returns the document as the codec read it.
    #[must_use]
    pub const fn document(&self) -> &Value {
        &self.document
    }

    /// Returns the `resourceType` the document declares.
    #[must_use]
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    /// Returns the `id` the sender wrote.
    #[must_use]
    pub const fn id(&self) -> Option<&ExternalResourceId> {
        self.id.as_ref()
    }

    /// Returns the `meta.versionId` the sender wrote.
    #[must_use]
    pub fn version_id(&self) -> Option<&str> {
        self.version_id.as_deref()
    }

    /// Returns the profiles the document claims, in document order.
    #[must_use]
    pub fn profiles(&self) -> &[String] {
        &self.profiles
    }

    /// Returns the subject the resource names.
    ///
    /// The EHR one composition is written into is resolved "by subject id and
    /// namespace" (`docs/architecture.md` §12), which is the pair the ITS-REST
    /// EHR lookup takes. A `subject.identifier` carries both halves; a literal
    /// `subject.reference` carries one, so `fallback` is the namespace a
    /// deployment gives its own references. No specification fixes the
    /// mapping from a FHIR subject to an openEHR subject: our own design.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::NoSubject`] when the resource names none and
    /// [`RequestError::Subject`] when the value cannot be a person key.
    pub fn subject(&self, fallback: &str) -> Result<PersonId, RequestError> {
        let reference = self
            .document
            .get("subject")
            .or_else(|| self.document.get("patient"))
            .ok_or(RequestError::NoSubject)?;
        if let Some(identifier) = reference.get("identifier") {
            let system = identifier
                .get("system")
                .and_then(Value::as_str)
                .unwrap_or(fallback);
            let value = identifier
                .get("value")
                .and_then(Value::as_str)
                .ok_or(RequestError::NoSubject)?;
            return PersonId::new(system, value).map_err(|source| RequestError::Subject { source });
        }
        let literal = reference
            .get("reference")
            .and_then(Value::as_str)
            .ok_or(RequestError::NoSubject)?;
        PersonId::new(fallback, literal).map_err(|source| RequestError::Subject { source })
    }
}

#[cfg(test)]
mod tests {
    use super::{Inbound, RequestError};

    /// The namespace a literal reference falls back to.
    const FALLBACK: &str = "http://example.org/fhir";

    /// Returns the bytes of `value`.
    fn body(value: &serde_json::Value) -> Vec<u8> {
        value.to_string().into_bytes()
    }

    #[test]
    fn a_resource_yields_its_type_id_version_and_profiles() {
        let read = Inbound::read(
            &body(&serde_json::json!({
                "resourceType": "Condition",
                "id": "c-1",
                "meta": {
                    "versionId": "2",
                    "profile": ["http://example.org/StructureDefinition/diagnosis"]
                },
                "subject": { "reference": "Patient/p-1" }
            })),
            "Condition",
        )
        .expect("a well-formed Condition reads");
        assert_eq!("Condition", read.resource_type());
        assert_eq!(
            Some("c-1"),
            read.id()
                .map(crate::facade::identity::ExternalResourceId::as_str)
        );
        assert_eq!(Some("2"), read.version_id());
        assert_eq!(
            vec![String::from(
                "http://example.org/StructureDefinition/diagnosis"
            )],
            read.profiles()
        );
    }

    #[test]
    fn a_body_declaring_another_type_than_the_path_is_refused() {
        let error = Inbound::read(
            &body(&serde_json::json!({ "resourceType": "Observation" })),
            "Condition",
        )
        .expect_err("the path and the body disagree");
        assert!(matches!(error, RequestError::WrongResourceType { .. }));
    }

    #[test]
    fn a_body_that_is_not_a_resource_object_is_refused() {
        assert!(matches!(
            Inbound::read(b"[]", "Condition"),
            Err(RequestError::NotAnObject)
        ));
        assert!(matches!(
            Inbound::read(b"{", "Condition"),
            Err(RequestError::NotJson { .. })
        ));
        assert!(matches!(
            Inbound::read(b"{\"id\":\"c-1\"}", "Condition"),
            Err(RequestError::NoResourceType)
        ));
    }

    #[test]
    fn the_subject_reads_from_an_identifier_and_falls_back_to_a_reference() {
        let identified = Inbound::read(
            &body(&serde_json::json!({
                "resourceType": "Condition",
                "subject": { "identifier": { "system": "http://example.org/mrn", "value": "p-1" } }
            })),
            "Condition",
        )
        .expect("the resource reads");
        let subject = identified.subject(FALLBACK).expect("a subject is named");
        assert_eq!("http://example.org/mrn", subject.namespace());
        assert_eq!("p-1", subject.id());

        let referenced = Inbound::read(
            &body(&serde_json::json!({
                "resourceType": "Condition",
                "subject": { "reference": "Patient/p-2" }
            })),
            "Condition",
        )
        .expect("the resource reads");
        let subject = referenced.subject(FALLBACK).expect("a subject is named");
        assert_eq!(FALLBACK, subject.namespace());
        assert_eq!("Patient/p-2", subject.id());
    }

    #[test]
    fn a_resource_whose_subject_is_optional_and_absent_is_refused_at_the_subject() {
        // R4 makes `Observation.subject` 0..1 and `Condition.subject` 1..1
        // (<https://hl7.org/fhir/R4/observation.html>), so this is the shape
        // where the codec admits a resource and the facade still has no EHR.
        let read = Inbound::read(
            &body(&serde_json::json!({
                "resourceType": "Observation",
                "status": "final",
                "code": { "text": "a synthetic measurement" }
            })),
            "Observation",
        )
        .expect("the resource reads");
        assert!(matches!(
            read.subject(FALLBACK),
            Err(RequestError::NoSubject)
        ));
    }

    #[test]
    fn a_required_element_the_body_omits_is_a_structural_refusal() {
        // `Condition.subject` is 1..1 (<https://hl7.org/fhir/R4/condition.html>).
        let error = Inbound::read(
            &body(&serde_json::json!({ "resourceType": "Condition" })),
            "Condition",
        )
        .expect_err("a Condition without a subject is refused");
        assert!(
            matches!(&error, RequestError::Structure { location, .. } if location == "Condition.subject"),
            "{error:?}"
        );
    }

    #[test]
    fn a_property_the_resource_does_not_define_is_a_structural_refusal() {
        let error = Inbound::read(
            &body(&serde_json::json!({
                "resourceType": "Condition",
                "subject": { "reference": "Patient/p-1" },
                "notAnElement": "x"
            })),
            "Condition",
        )
        .expect_err("an unknown property is refused, never dropped");
        assert!(
            matches!(&error, RequestError::Structure { reason, .. } if reason.contains("unknown property")),
            "{error:?}"
        );
    }
}
