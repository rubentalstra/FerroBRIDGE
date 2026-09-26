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
/// resource resolves to the composition it already produced, and a later
/// version of the same `id` revises that composition (no specification
/// governs this: our own design).
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

/// The type-slot prefix of a message entry key, a colon no FHIR resource type
/// carries.
const MESSAGE_TYPE_PREFIX: &str = "message:";

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
            resource_type: format!("{MESSAGE_TYPE_PREFIX}{checked}"),
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

    /// Returns the key of the source resource whatever its version: the
    /// resource type and the `id`, joined by the unit separator.
    ///
    /// A [`storage_key`](Self::storage_key) carries two separators and this
    /// key one, so the two never meet in one claim set. No specification
    /// governs the key shape: our own design.
    #[must_use]
    pub fn resource_key(&self) -> String {
        format!("{}\u{1f}{}", self.resource_type, self.id)
    }

    /// Returns the key one delivery may carry once.
    ///
    /// A resource appears in a transaction once by identity, whatever its
    /// `meta.versionId` (<https://hl7.org/fhir/R4/http.html#transaction>), so
    /// its key is the [`resource_key`](Self::resource_key). Each entry of one
    /// message is a source of its own, so a message entry keeps its whole
    /// [`storage_key`](Self::storage_key).
    #[must_use]
    pub fn once_key(&self) -> String {
        if self.resource_type.starts_with(MESSAGE_TYPE_PREFIX) {
            self.storage_key()
        } else {
            self.resource_key()
        }
    }

    /// Returns the range of storage keys that holds every version of this
    /// source's `id`, with a `meta.versionId` or without one.
    ///
    /// Every such key starts with the resource type, the `id` and a
    /// separator, and `U+0020` is the character after `U+001F`, so the range
    /// ends before the key of any other `id`.
    #[must_use]
    pub fn every_version(&self) -> core::ops::Range<String> {
        let resource = self.resource_key();
        format!("{resource}\u{1f}")..format!("{resource}\u{20}")
    }
}

/// One `Resource.identifier` a sender wrote: its `system`, when it names one,
/// and its `value`.
///
/// The conditional create answers an `identifier` search from these
/// (<https://hl7.org/fhir/R4/search.html#token>), so every committed resource
/// records each identifier it carries against its logical id. An identifier
/// is a business identifier the sender assigned, never a clinical value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier {
    /// The `Identifier.system`, when the sender wrote one.
    system: Option<String>,
    /// The `Identifier.value`.
    value: String,
}

impl Identifier {
    /// Returns the identifier `value` in `system`.
    ///
    /// An empty `system` is read as no system, since the R4 `uri` type admits
    /// no empty value (<https://hl7.org/fhir/R4/datatypes.html#uri>).
    #[must_use]
    pub fn new(system: Option<&str>, value: &str) -> Self {
        Self {
            system: system
                .filter(|system| !system.is_empty())
                .map(str::to_owned),
            value: value.to_owned(),
        }
    }

    /// Returns the `Identifier.system`, when the sender wrote one.
    #[must_use]
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    /// Returns the `Identifier.value`.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the key this identifier of `internal` is stored under.
    ///
    /// The key is the resource type, then the value and the system, each
    /// framed by its byte length, then the logical id. So every resource that
    /// carries one identifier has a key of its own, and a search reads them
    /// all ([`IdentifierQuery::range`]). The framing keeps a value or a
    /// system that carries a separator from borrowing its neighbour. No
    /// specification governs the key shape: our own design.
    #[must_use]
    pub fn storage_key(&self, resource_type: &str, internal: &FhirResourceId) -> String {
        let system = self.system.as_deref().unwrap_or_default();
        format!(
            "{}\u{1f}{internal}",
            framed(resource_type, &self.value, Some(system))
        )
    }
}

/// Which `Identifier.system` an `identifier` search admits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemMatch {
    /// `[code]`: any system, or none.
    Any,
    /// `|[code]`: only an identifier that names no system.
    Absent,
    /// `[system]|[code]`: only this system.
    Is(String),
}

/// One value of an `identifier` search parameter, read as an R4 token.
///
/// The token forms are `[code]`, `|[code]` and `[system]|[code]`
/// (<https://hl7.org/fhir/R4/search.html#token>).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierQuery {
    /// Which system the search admits.
    system: SystemMatch,
    /// The value the search matches exactly.
    value: String,
}

impl IdentifierQuery {
    /// Returns the query that matches `value` under `system`.
    #[must_use]
    pub fn new(system: SystemMatch, value: &str) -> Self {
        Self {
            system,
            value: value.to_owned(),
        }
    }

    /// Returns the range of storage keys that holds every identifier of
    /// `resource_type` this query matches.
    ///
    /// Under [`SystemMatch::Any`] the range is every key that continues the
    /// framed value, whose next character is a digit of the system's length;
    /// `:` follows the digits, so the range ends before any other value.
    /// Otherwise the range is every key that continues the framed system with
    /// the separator `U+001F`, and `U+0020` is the character after it.
    #[must_use]
    pub fn range(&self, resource_type: &str) -> core::ops::Range<String> {
        let system = match self.system {
            SystemMatch::Any => {
                let prefix = framed(resource_type, &self.value, None);
                let end = format!("{prefix}:");
                return prefix..end;
            }
            SystemMatch::Absent => "",
            SystemMatch::Is(ref system) => system.as_str(),
        };
        let prefix = framed(resource_type, &self.value, Some(system));
        format!("{prefix}\u{1f}")..format!("{prefix}\u{20}")
    }
}

/// Returns the resource type, the length-framed value and, when given, the
/// length-framed system, as an identifier key starts.
fn framed(resource_type: &str, value: &str, system: Option<&str>) -> String {
    let head = format!("{resource_type}\u{1f}{}:{value}", value.len());
    match system {
        Some(system) => format!("{head}{}:{system}", system.len()),
        None => head,
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
    fn every_version_of_one_id_and_no_other_id_falls_in_its_range() {
        let source = SourceVersion::new(
            "Condition",
            ExternalResourceId::new("c-1").expect("a legal external id"),
            Some(String::from("2")),
        );
        let range = source.every_version();
        let key = |id: &str, version: Option<&str>| {
            SourceVersion::new(
                "Condition",
                ExternalResourceId::new(id).expect("a legal external id"),
                version.map(String::from),
            )
            .storage_key()
        };
        assert!(range.contains(&key("c-1", Some("1"))));
        assert!(range.contains(&key("c-1", Some("2"))));
        assert!(range.contains(&key("c-1", None)));
        assert!(!range.contains(&key("c-10", Some("1"))));
        assert!(!range.contains(&key("c-", None)));
        assert_ne!(source.resource_key(), key("c-1", None));
    }

    #[test]
    fn a_resource_is_once_by_id_and_a_message_entry_by_position() {
        let id = || ExternalResourceId::new("c-1").expect("a legal external id");
        let one = SourceVersion::new("Condition", id(), Some(String::from("1")));
        let two = SourceVersion::new("Condition", id(), Some(String::from("2")));
        assert_eq!(one.once_key(), two.once_key());
        let control = || ExternalResourceId::new("MSG-0001").expect("a legal external id");
        let first =
            SourceVersion::of_message_entry("ORU^R01", control(), 0).expect("a legal message type");
        let second =
            SourceVersion::of_message_entry("ORU^R01", control(), 1).expect("a legal message type");
        assert_ne!(first.once_key(), second.once_key());
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
    fn an_identifier_query_reads_exactly_the_keys_its_token_form_admits() {
        use super::{Identifier, IdentifierQuery, SystemMatch};
        let id = |text: &str| FhirResourceId::new(text).expect("a legal FHIR id");
        let key = |system: Option<&str>, value: &str, internal: &str| {
            Identifier::new(system, value).storage_key("Condition", &id(internal))
        };
        let held = [
            key(Some("http://a"), "v-1", "one"),
            key(Some("http://b"), "v-1", "two"),
            key(None, "v-1", "three"),
            key(Some("http://a"), "v-10", "four"),
            key(Some("http://a"), "v-1", "five"),
        ];
        let matching = |query: IdentifierQuery| -> Vec<usize> {
            let range = query.range("Condition");
            (0..held.len())
                .filter(|&i| range.contains(&held[i]))
                .collect()
        };
        assert_eq!(
            vec![0, 1, 2, 4],
            matching(IdentifierQuery::new(SystemMatch::Any, "v-1"))
        );
        assert_eq!(
            vec![0, 4],
            matching(IdentifierQuery::new(
                SystemMatch::Is(String::from("http://a")),
                "v-1"
            ))
        );
        assert_eq!(
            vec![2],
            matching(IdentifierQuery::new(SystemMatch::Absent, "v-1"))
        );
        assert!(
            !IdentifierQuery::new(SystemMatch::Any, "v-1")
                .range("Observation")
                .contains(&held[0]),
            "an identifier of one type never answers a search of another"
        );
    }

    #[test]
    fn a_separator_inside_an_identifier_cannot_borrow_its_neighbour() {
        use super::{Identifier, IdentifierQuery, SystemMatch};
        let internal = FhirResourceId::new("one").expect("a legal FHIR id");
        let held = Identifier::new(Some("b"), "a1:").storage_key("Condition", &internal);
        assert!(
            !IdentifierQuery::new(SystemMatch::Any, "a")
                .range("Condition")
                .contains(&held)
        );
        assert!(
            !IdentifierQuery::new(SystemMatch::Is(String::from("1:b")), "a")
                .range("Condition")
                .contains(&held)
        );
        assert_eq!(None, Identifier::new(Some(""), "a").system());
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
