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
