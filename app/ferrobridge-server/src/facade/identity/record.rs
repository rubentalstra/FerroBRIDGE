// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the identity store holds, and what it deliberately does not.
//!
//! Every field below is an identifier, a path or a mapping name. No clinical
//! value, no narrative and no subject attribute reaches this module, which is
//! what `the_store_never_holds_clinical_content` asserts against the store
//! file itself (`docs/architecture.md` §9).

use serde::Deserialize;
use serde::Serialize;

use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::IdError;

/// Where one FHIR resource lives in the CDR.
///
/// The FHIR side addresses a resource by its logical id; the openEHR side
/// addresses the same content by an EHR, a version container and the path of
/// one entry inside it, so this record is the whole translation
/// (`docs/architecture.md` §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionBinding {
    /// The EHR the composition lives in.
    pub ehr_id: String,
    /// The version container of the composition.
    pub versioned_object_uid: String,
    /// The template the composition was committed against.
    pub template_id: String,
    /// The FHIR resource type the entry maps to.
    pub resource_type: String,
    /// The archetype path of the entry inside the composition.
    pub entry_path: String,
    /// Which document of a `hierarchy` split the entry belongs to.
    pub split: u32,
    /// The `metadata.name` of the context mapping that produced the resource.
    pub context: String,
}

/// The mapping one inbound resource version was consumed by.
///
/// The key is the sending system's `id` and `meta.versionId`, so a re-sent
/// resource resolves to the composition it already produced and updates it
/// rather than creating a second one (`docs/architecture.md` §4.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumedSource {
    /// The EHR the composition was committed into.
    pub ehr_id: String,
    /// The version container the commit produced.
    pub versioned_object_uid: String,
    /// The logical id the bridge gave the resource.
    pub internal_id: String,
    /// The `metadata.name` of the context mapping that consumed the resource.
    pub context: String,
}

/// The contribution one inbound source version was committed in, recorded
/// before its composition is bound.
///
/// A CONTRIBUTION is committed before the service can bind its versions to
/// the entries it carried, so this record is what a re-sent Bundle meets when
/// the binding of its first delivery failed: the bridge reads the contribution
/// back by this uid and commits nothing a second time. No specification
/// governs this record: our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedSource {
    /// The EHR the contribution was committed into.
    pub ehr_id: String,
    /// The `contribution_uid` the CDR named in its answer.
    pub contribution_uid: String,
}

/// The key of one inbound resource version.
///
/// A resource that arrives without a `meta.versionId` still has an identity,
/// so the absent version is part of the key rather than a reason to skip the
/// lookup: the same resource re-sent without a version is the same key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceVersion {
    /// The resource type the sender declared.
    resource_type: String,
    /// The `id` the sender wrote.
    id: ExternalResourceId,
    /// The `meta.versionId` the sender wrote, when it wrote one.
    version_id: Option<String>,
}

impl SourceVersion {
    /// Returns the key of `id` at `version_id` for `resource_type`.
    #[must_use]
    pub fn new(
        resource_type: impl Into<String>,
        id: ExternalResourceId,
        version_id: Option<String>,
    ) -> Self {
        Self {
            resource_type: resource_type.into(),
            id,
            version_id,
        }
    }

    /// Returns the key of the entry at `position` of the Bundle one message
    /// stands for.
    ///
    /// One message can carry several resources, so the key is the message's
    /// type and control id with the entry's Bundle position in the version
    /// slot, and a redelivered message meets the keys its first delivery
    /// recorded. The type slot reads `message:` before the message type, a
    /// colon no FHIR resource type carries, so a message key never meets a
    /// resource key. No specification governs this key: our own design.
    ///
    /// # Errors
    ///
    /// Returns [`IdError`] when the message type is empty or carries a
    /// control character, which the key's separator could then collide with.
    pub fn of_message_entry(
        message_type: &str,
        control_id: ExternalResourceId,
        position: usize,
    ) -> Result<Self, IdError> {
        let checked = ExternalResourceId::new(message_type)?;
        Ok(Self {
            resource_type: format!("message:{checked}"),
            id: control_id,
            version_id: Some(format!("entry-{position}")),
        })
    }

    /// Returns the resource type the sender declared.
    #[must_use]
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    /// Returns the `id` the sender wrote.
    #[must_use]
    pub const fn id(&self) -> &ExternalResourceId {
        &self.id
    }

    /// Returns the `meta.versionId` the sender wrote.
    #[must_use]
    pub fn version_id(&self) -> Option<&str> {
        self.version_id.as_deref()
    }

    /// Returns the key this pair is stored under.
    ///
    /// The three parts are joined by a unit separator (`U+001F`), which no
    /// part can carry: a resource type is a FHIR name, an
    /// [`ExternalResourceId`] refuses every control character, and a
    /// `meta.versionId` is a FHIR `id`. No specification governs the key
    /// shape: our own design.
    #[must_use]
    pub fn storage_key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.resource_type,
            self.id,
            self.version_id.as_deref().unwrap_or_default()
        )
    }
}

/// Returns the storage key of an internal resource id.
///
/// The type joins the id so two resource types cannot collide on one logical
/// id, which R4 allows: an id is unique inside its type, not across the server
/// (<https://hl7.org/fhir/R4/resource.html>).
#[must_use]
pub fn internal_key(resource_type: &str, id: &FhirResourceId) -> String {
    format!("{resource_type}\u{1f}{id}")
}

/// Returns the storage key of an external resource id.
#[must_use]
pub fn external_key(resource_type: &str, id: &ExternalResourceId) -> String {
    format!("{resource_type}\u{1f}{id}")
}

#[cfg(test)]
mod tests {
    use super::{SourceVersion, external_key, internal_key};
    use crate::facade::identity::{ExternalResourceId, FhirResourceId};

    #[test]
    fn a_source_key_separates_its_three_parts() {
        let id = ExternalResourceId::new("c-1").expect("a legal external id");
        let with = SourceVersion::new("Condition", id.clone(), Some(String::from("2")));
        let without = SourceVersion::new("Condition", id, None);
        assert_eq!("Condition\u{1f}c-1\u{1f}2", with.storage_key());
        assert_eq!("Condition\u{1f}c-1\u{1f}", without.storage_key());
        assert_ne!(with.storage_key(), without.storage_key());
    }

    #[test]
    fn a_message_entry_key_never_meets_a_resource_key() {
        let control = ExternalResourceId::new("MSG-0001").expect("a legal external id");
        let entry = SourceVersion::of_message_entry("ORU^R01", control.clone(), 2)
            .expect("a legal message type");
        assert_eq!(
            "message:ORU^R01\u{1f}MSG-0001\u{1f}entry-2",
            entry.storage_key()
        );
        let other = SourceVersion::of_message_entry("ORU^R01", control.clone(), 3)
            .expect("a legal message type");
        assert_ne!(entry.storage_key(), other.storage_key());
        assert!(SourceVersion::of_message_entry("ORU\u{1f}R01", control, 2).is_err());
    }

    #[test]
    fn two_types_sharing_one_logical_id_are_two_keys() {
        let internal = FhirResourceId::new("abc").expect("a legal FHIR id");
        assert_ne!(
            internal_key("Condition", &internal),
            internal_key("Observation", &internal)
        );
        let external = ExternalResourceId::new("abc").expect("a legal external id");
        assert_ne!(
            external_key("Condition", &external),
            external_key("Observation", &external)
        );
    }
}
