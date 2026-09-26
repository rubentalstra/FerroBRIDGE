// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identity map: the seam, and the in-memory implementation tests use.
//!
//! Four tables, as `docs/architecture.md` §9 names them: a patient identifier
//! to an `ehr_id`, an external resource id to the internal one, an internal
//! resource id to the composition it lives in, and a source resource `id` with
//! its `meta.versionId` to the mapping that consumed them. A fifth table keys
//! the same source to the CONTRIBUTION that committed it, written before the
//! binding so a re-sent Bundle is recognised when its first binding failed. A
//! sixth keys each `Resource.identifier` a committed resource carries to its
//! logical id, which is what the conditional create's `identifier` search
//! reads. A seventh keys each `ehr_id` back to the person it was recorded
//! for, which is what a read writes a resource's subject from.
//!
//! Every write is record-once. "Once assigned, this value never changes"
//! (<https://hl7.org/fhir/R4/resource.html>), so a `record_*` call answers with
//! the binding that now stands: the stored one when the table already held it,
//! the offered one otherwise. A caller cannot overwrite an identity by
//! mistake, because no method offers to.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::cdr::ids::EhrId;

use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::PersonId;
use crate::facade::identity::record::CommittedSource;
use crate::facade::identity::record::CompositionBinding;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::record::Identifier;
use crate::facade::identity::record::IdentifierQuery;
use crate::facade::identity::record::SourceVersion;
use crate::facade::identity::record::external_key;
use crate::facade::identity::record::internal_key;

/// Why the identity store could not answer.
///
/// Every variant carries its cause, so a `502` from the facade names the layer
/// that refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The store file could not be opened or created.
    #[error("the identity store at {path} could not be opened")]
    Open {
        /// The path that was tried.
        path: String,
        /// What the storage layer reported.
        #[source]
        source: Box<redb::DatabaseError>,
    },
    /// A transaction could not be started or committed.
    #[error("an identity store transaction did not complete")]
    Transaction {
        /// What the storage layer reported.
        #[source]
        source: Box<redb::Error>,
    },
    /// A stored record is not the shape this version writes.
    #[error("the identity store holds a record under {key} that this version cannot read")]
    Record {
        /// The key the record sits under.
        key: String,
        /// What the decoder reported.
        #[source]
        source: Box<serde_json::Error>,
    },
    /// A stored identifier no longer passes its own grammar.
    #[error("the identity store holds {value} under {key}, which is no legal {kind}")]
    Identifier {
        /// The key the record sits under.
        key: String,
        /// What kind of identifier was expected.
        kind: &'static str,
        /// The value that was read.
        value: String,
        /// What the identifier's own constructor reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A write committed and the key it wrote read back empty.
    #[error("the identity store did not hold {key} after writing it")]
    Missing {
        /// The key that was written.
        key: String,
    },
    /// The in-memory store's lock was poisoned by a panicking writer.
    #[error("the in-memory identity store is poisoned")]
    Poisoned,
}

/// The identity map the facade resolves every request through.
///
/// The trait is synchronous: `redb` is an embedded store with no I/O wait of
/// its own, and a handler calls it between two awaits rather than across one,
/// so no blocking call hides inside an await.
pub trait Store: fmt::Debug + Send + Sync {
    /// Returns the EHR recorded for `patient`, when one is.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read.
    fn ehr_of(&self, patient: &PersonId) -> Result<Option<EhrId>, StoreError>;

    /// Records `ehr` as the EHR of `patient`, and returns the one that stands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or written.
    fn record_ehr(&self, patient: &PersonId, ehr: &EhrId) -> Result<EhrId, StoreError>;

    /// Returns the person `ehr` was first recorded for, when the map knows.
    ///
    /// [`Store::record_ehr`] writes the reverse row in the same step, record
    /// once like the forward one, so the first person recorded for an EHR
    /// stands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or holds a person
    /// key this version cannot read.
    fn person_of(&self, ehr: &EhrId) -> Result<Option<PersonId>, StoreError>;

    /// Returns the internal id recorded for a sender's id, when one is.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read.
    fn internal_of(
        &self,
        resource_type: &str,
        external: &ExternalResourceId,
    ) -> Result<Option<FhirResourceId>, StoreError>;

    /// Records `internal` as the id of `external`, and returns the one that
    /// stands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or written.
    fn record_internal(
        &self,
        resource_type: &str,
        external: &ExternalResourceId,
        internal: &FhirResourceId,
    ) -> Result<FhirResourceId, StoreError>;

    /// Returns where `internal` lives in the CDR, when the map knows.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read.
    fn binding_of(
        &self,
        resource_type: &str,
        internal: &FhirResourceId,
    ) -> Result<Option<CompositionBinding>, StoreError>;

    /// Records where `internal` lives, and returns the binding that stands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or written.
    fn record_binding(
        &self,
        resource_type: &str,
        internal: &FhirResourceId,
        binding: &CompositionBinding,
    ) -> Result<CompositionBinding, StoreError>;

    /// Returns what consumed `source`, when the map knows.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read.
    fn consumed(&self, source: &SourceVersion) -> Result<Option<ConsumedSource>, StoreError>;

    /// Returns what consumed every version of the `id` `source` names, with
    /// a `meta.versionId` or without one, in storage-key order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read.
    fn consumed_versions(&self, source: &SourceVersion) -> Result<Vec<ConsumedSource>, StoreError>;

    /// Records what consumed `source`, and returns the record that stands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or written.
    fn record_consumed(
        &self,
        source: &SourceVersion,
        consumed: &ConsumedSource,
    ) -> Result<ConsumedSource, StoreError>;

    /// Returns the contribution `source` was committed in, when the map
    /// knows.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read.
    fn committed(&self, source: &SourceVersion) -> Result<Option<CommittedSource>, StoreError>;

    /// Records the contribution `source` was committed in, and returns the
    /// record that stands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or written.
    fn record_committed(
        &self,
        source: &SourceVersion,
        committed: &CommittedSource,
    ) -> Result<CommittedSource, StoreError>;

    /// Returns the logical id of every resource of `resource_type` that
    /// carries an identifier `query` matches, in storage-key order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be read or holds an id
    /// this version cannot read.
    fn identified(
        &self,
        resource_type: &str,
        query: &IdentifierQuery,
    ) -> Result<Vec<FhirResourceId>, StoreError>;

    /// Records that the resource `internal` carries `identifier`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the store cannot be written.
    fn record_identifier(
        &self,
        resource_type: &str,
        identifier: &Identifier,
        internal: &FhirResourceId,
    ) -> Result<(), StoreError>;
}

/// Returns the logical id one identifier row holds.
pub(crate) fn identified_id(key: &str, value: &str) -> Result<FhirResourceId, StoreError> {
    FhirResourceId::new(value).map_err(|source| StoreError::Identifier {
        key: key.to_owned(),
        kind: "FHIR id",
        value: value.to_owned(),
        source: Box::new(source),
    })
}

/// The tables, as one map each.
#[derive(Debug, Default)]
struct Tables {
    /// Patient identifier to `ehr_id`.
    patients: BTreeMap<String, String>,
    /// `ehr_id` to the person it was first recorded for.
    subjects: BTreeMap<String, PersonId>,
    /// External resource id to internal resource id.
    external: BTreeMap<String, String>,
    /// Internal resource id to its composition binding.
    bindings: BTreeMap<String, CompositionBinding>,
    /// Source resource `id` and `meta.versionId` to what consumed them.
    sources: BTreeMap<String, ConsumedSource>,
    /// Source resource `id` and `meta.versionId` to the contribution that
    /// committed them.
    contributions: BTreeMap<String, CommittedSource>,
    /// Resource type, identifier and logical id to that logical id.
    identifiers: BTreeMap<String, String>,
}

/// The identity map a test drives, held in memory and lost with the process.
#[derive(Debug, Default)]
pub struct MemoryStore {
    /// The seven tables behind one lock, so a write is atomic across them.
    tables: Mutex<Tables>,
}

impl MemoryStore {
    /// Returns an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `f` over the tables.
    fn with<T>(&self, f: impl FnOnce(&mut Tables) -> T) -> Result<T, StoreError> {
        let mut guard = self
            .tables
            .lock()
            .map_err(|_poisoned| StoreError::Poisoned)?;
        Ok(f(&mut guard))
    }
}

impl Store for MemoryStore {
    fn ehr_of(&self, patient: &PersonId) -> Result<Option<EhrId>, StoreError> {
        let key = patient.to_string();
        let found = self.with(|tables| tables.patients.get(&key).cloned())?;
        found
            .map(|value| {
                EhrId::new(&value).map_err(|source| StoreError::Identifier {
                    key: key.clone(),
                    kind: "ehr_id",
                    value,
                    source: Box::new(source),
                })
            })
            .transpose()
    }

    fn record_ehr(&self, patient: &PersonId, ehr: &EhrId) -> Result<EhrId, StoreError> {
        if let Some(stored) = self.ehr_of(patient)? {
            return Ok(stored);
        }
        let key = patient.to_string();
        let value = ehr.as_str().to_owned();
        self.with(|tables| {
            tables
                .subjects
                .entry(value.clone())
                .or_insert_with(|| patient.clone());
            tables.patients.insert(key, value)
        })?;
        Ok(ehr.clone())
    }

    fn person_of(&self, ehr: &EhrId) -> Result<Option<PersonId>, StoreError> {
        self.with(|tables| tables.subjects.get(ehr.as_str()).cloned())
    }

    fn internal_of(
        &self,
        resource_type: &str,
        external: &ExternalResourceId,
    ) -> Result<Option<FhirResourceId>, StoreError> {
        let key = external_key(resource_type, external);
        let found = self.with(|tables| tables.external.get(&key).cloned())?;
        found
            .map(|value| {
                FhirResourceId::new(&value).map_err(|source| StoreError::Identifier {
                    key: key.clone(),
                    kind: "FHIR id",
                    value,
                    source: Box::new(source),
                })
            })
            .transpose()
    }

    fn record_internal(
        &self,
        resource_type: &str,
        external: &ExternalResourceId,
        internal: &FhirResourceId,
    ) -> Result<FhirResourceId, StoreError> {
        if let Some(stored) = self.internal_of(resource_type, external)? {
            return Ok(stored);
        }
        let key = external_key(resource_type, external);
        let value = internal.as_str().to_owned();
        self.with(|tables| tables.external.insert(key, value))?;
        Ok(internal.clone())
    }

    fn binding_of(
        &self,
        resource_type: &str,
        internal: &FhirResourceId,
    ) -> Result<Option<CompositionBinding>, StoreError> {
        let key = internal_key(resource_type, internal);
        self.with(|tables| tables.bindings.get(&key).cloned())
    }

    fn record_binding(
        &self,
        resource_type: &str,
        internal: &FhirResourceId,
        binding: &CompositionBinding,
    ) -> Result<CompositionBinding, StoreError> {
        let key = internal_key(resource_type, internal);
        self.with(|tables| {
            tables
                .bindings
                .entry(key)
                .or_insert_with(|| binding.clone())
                .clone()
        })
    }

    fn consumed(&self, source: &SourceVersion) -> Result<Option<ConsumedSource>, StoreError> {
        let key = source.storage_key();
        self.with(|tables| tables.sources.get(&key).cloned())
    }

    fn consumed_versions(&self, source: &SourceVersion) -> Result<Vec<ConsumedSource>, StoreError> {
        let range = source.every_version();
        self.with(|tables| {
            tables
                .sources
                .range(range)
                .map(|(_, held)| held.clone())
                .collect()
        })
    }

    fn record_consumed(
        &self,
        source: &SourceVersion,
        consumed: &ConsumedSource,
    ) -> Result<ConsumedSource, StoreError> {
        let key = source.storage_key();
        self.with(|tables| {
            tables
                .sources
                .entry(key)
                .or_insert_with(|| consumed.clone())
                .clone()
        })
    }

    fn committed(&self, source: &SourceVersion) -> Result<Option<CommittedSource>, StoreError> {
        let key = source.storage_key();
        self.with(|tables| tables.contributions.get(&key).cloned())
    }

    fn record_committed(
        &self,
        source: &SourceVersion,
        committed: &CommittedSource,
    ) -> Result<CommittedSource, StoreError> {
        let key = source.storage_key();
        self.with(|tables| {
            tables
                .contributions
                .entry(key)
                .or_insert_with(|| committed.clone())
                .clone()
        })
    }

    fn identified(
        &self,
        resource_type: &str,
        query: &IdentifierQuery,
    ) -> Result<Vec<FhirResourceId>, StoreError> {
        let range = query.range(resource_type);
        let rows: Vec<(String, String)> = self.with(|tables| {
            tables
                .identifiers
                .range(range)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })?;
        rows.iter()
            .map(|(key, value)| identified_id(key, value))
            .collect()
    }

    fn record_identifier(
        &self,
        resource_type: &str,
        identifier: &Identifier,
        internal: &FhirResourceId,
    ) -> Result<(), StoreError> {
        let key = identifier.storage_key(resource_type, internal);
        let value = String::from(internal.as_str());
        self.with(|tables| {
            tables.identifiers.entry(key).or_insert(value);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryStore, Store};
    use crate::cdr::ids::EhrId;
    use crate::facade::identity::record::{CompositionBinding, ConsumedSource, SourceVersion};
    use crate::facade::identity::{ExternalResourceId, FhirResourceId, PersonId};

    /// Returns a binding naming `uid` as its version container.
    fn binding(uid: &str) -> CompositionBinding {
        CompositionBinding {
            ehr_id: String::from("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11"),
            versioned_object_uid: String::from(uid),
            template_id: String::from("ferrobridge.diagnose.v1"),
            resource_type: String::from("Condition"),
            entry_path: String::from("/content[0]"),
            split: 0,
            context: String::from("ferrobridge_diagnosis.context"),
        }
    }

    #[test]
    fn an_empty_store_knows_nothing() {
        let store = MemoryStore::new();
        let patient = PersonId::new("http://example.org/ns", "p-1").expect("a legal person id");
        assert!(
            store
                .ehr_of(&patient)
                .expect("a read of an empty store")
                .is_none()
        );
    }

    #[test]
    fn the_map_wins_once_written() {
        let store = MemoryStore::new();
        let internal = FhirResourceId::new("abc").expect("a legal FHIR id");
        let first = store
            .record_binding("Condition", &internal, &binding("uid-1"))
            .expect("the first write");
        let second = store
            .record_binding("Condition", &internal, &binding("uid-2"))
            .expect("the second write");
        assert_eq!(first, second, "a second write does not move the binding");
        assert_eq!("uid-1", second.versioned_object_uid);
    }

    #[test]
    fn a_recorded_ehr_and_internal_id_read_back() {
        let store = MemoryStore::new();
        let patient = PersonId::new("http://example.org/ns", "p-1").expect("a legal person id");
        let ehr = EhrId::new("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11").expect("a legal ehr_id");
        assert_eq!(ehr, store.record_ehr(&patient, &ehr).expect("the write"));
        assert_eq!(Some(ehr.clone()), store.ehr_of(&patient).expect("the read"));
        assert_eq!(
            Some(patient),
            store.person_of(&ehr).expect("the reverse read")
        );
        let unknown = EhrId::new("0f4b3a0c-9f11-4a4a-9e0b-bd6b1e5a3b9b").expect("a legal ehr_id");
        assert_eq!(None, store.person_of(&unknown).expect("the reverse read"));

        let external = ExternalResourceId::new("sender-1").expect("a legal external id");
        let internal = FhirResourceId::new("abc").expect("a legal FHIR id");
        assert_eq!(
            internal,
            store
                .record_internal("Condition", &external, &internal)
                .expect("the write")
        );
        assert_eq!(
            Some(internal),
            store.internal_of("Condition", &external).expect("the read")
        );
    }

    #[test]
    fn a_consumed_source_resolves_a_re_sent_resource() {
        let store = MemoryStore::new();
        let external = ExternalResourceId::new("sender-1").expect("a legal external id");
        let source = SourceVersion::new("Condition", external, Some(String::from("1")));
        let consumed = ConsumedSource {
            ehr_id: String::from("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11"),
            versioned_object_uid: String::from("uid-1"),
            internal_id: String::from("abc"),
            context: String::from("ferrobridge_diagnosis.context"),
        };
        assert!(store.consumed(&source).expect("an empty read").is_none());
        store
            .record_consumed(&source, &consumed)
            .expect("the write");
        assert_eq!(
            Some(consumed),
            store.consumed(&source).expect("the read back")
        );
    }

    #[test]
    fn every_consumed_version_of_one_id_reads_back_and_no_other_id() {
        let store = MemoryStore::new();
        let source = |id: &str, version: Option<&str>| {
            SourceVersion::new(
                "Condition",
                ExternalResourceId::new(id).expect("a legal external id"),
                version.map(String::from),
            )
        };
        let consumed = |uid: &str| ConsumedSource {
            ehr_id: String::from("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11"),
            versioned_object_uid: String::from(uid),
            internal_id: String::from("abc"),
            context: String::from("ferrobridge_diagnosis.context"),
        };
        for (id, version, uid) in [
            ("c-1", Some("1"), "uid-1"),
            ("c-1", None, "uid-2"),
            ("c-10", Some("1"), "uid-3"),
        ] {
            store
                .record_consumed(&source(id, version), &consumed(uid))
                .expect("the write");
        }
        assert_eq!(
            vec![consumed("uid-2"), consumed("uid-1")],
            store
                .consumed_versions(&source("c-1", Some("9")))
                .expect("the read")
        );
    }
}
