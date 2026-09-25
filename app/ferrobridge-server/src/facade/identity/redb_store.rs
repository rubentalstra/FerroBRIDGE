// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identity map on disk, over `redb`.
//!
//! Four `redb` tables, one per row of `docs/architecture.md` §9, and a fifth
//! from a source to the contribution that committed it. The file
//! holds identifiers, archetype paths and mapping names and nothing else: no
//! clinical value ever reaches it, which
//! `the_store_never_holds_clinical_content` asserts by reading the file back
//! as bytes.
//!
//! `redb` is an ACID embedded store (<https://docs.rs/redb/4.3.0/redb/>), so a
//! record-once write is one write transaction that reads the key first and
//! commits only when the key was free.

use std::path::Path;
use std::path::PathBuf;

use crate::cdr::ids::EhrId;
use redb::Database;
use redb::ReadableDatabase;
use redb::ReadableTable;
use redb::TableDefinition;

use crate::facade::identity::ExternalResourceId;
use crate::facade::identity::FhirResourceId;
use crate::facade::identity::PersonId;
use crate::facade::identity::record::CommittedSource;
use crate::facade::identity::record::CompositionBinding;
use crate::facade::identity::record::ConsumedSource;
use crate::facade::identity::record::SourceVersion;
use crate::facade::identity::record::external_key;
use crate::facade::identity::record::internal_key;
use crate::facade::identity::store::Store;
use crate::facade::identity::store::StoreError;

/// Patient identifier to `ehr_id`.
const PATIENTS: TableDefinition<'static, &str, &str> = TableDefinition::new("patient_ehr");

/// External resource id to internal resource id.
const EXTERNAL: TableDefinition<'static, &str, &str> = TableDefinition::new("external_internal");

/// Internal resource id to the composition binding, as JSON.
const BINDINGS: TableDefinition<'static, &str, &str> = TableDefinition::new("internal_composition");

/// Source resource `id` and `meta.versionId` to what consumed them, as JSON.
const SOURCES: TableDefinition<'static, &str, &str> = TableDefinition::new("source_mapping");

/// Source resource `id` and `meta.versionId` to the contribution that
/// committed them, as JSON.
const CONTRIBUTIONS: TableDefinition<'static, &str, &str> =
    TableDefinition::new("source_contribution");

/// The identity map of one configured CDR, on disk.
///
/// One store per configured CDR: a multi-tenant deployment runs one bridge per
/// tenant (`docs/architecture.md` §9 §Tenancy).
#[derive(Debug)]
pub struct RedbStore {
    /// The open database.
    database: Database,
    /// The path it was opened at, for a diagnostic that names the file.
    path: PathBuf,
}

impl RedbStore {
    /// Opens, or creates, the store at `path`.
    ///
    /// The four tables are created in one transaction, so every later read
    /// finds a table rather than a missing one.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Open`] when the path cannot be opened or created
    /// and [`StoreError::Transaction`] when the table creation does not
    /// commit.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let database = Database::create(path).map_err(|source| StoreError::Open {
            path: path.display().to_string(),
            source: Box::new(source),
        })?;
        let store = Self {
            database,
            path: path.to_path_buf(),
        };
        let write = store.database.begin_write().map_err(transaction)?;
        {
            let _patients = write.open_table(PATIENTS).map_err(transaction)?;
            let _external = write.open_table(EXTERNAL).map_err(transaction)?;
            let _bindings = write.open_table(BINDINGS).map_err(transaction)?;
            let _sources = write.open_table(SOURCES).map_err(transaction)?;
            let _contributions = write.open_table(CONTRIBUTIONS).map_err(transaction)?;
        }
        write.commit().map_err(transaction)?;
        Ok(store)
    }

    /// Returns the path the store was opened at.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the value `key` holds in `table`.
    fn get(
        &self,
        table: TableDefinition<'static, &str, &str>,
        key: &str,
    ) -> Result<Option<String>, StoreError> {
        let read = self.database.begin_read().map_err(transaction)?;
        let opened = read.open_table(table).map_err(transaction)?;
        let found = opened.get(key).map_err(transaction)?;
        Ok(found.map(|value| value.value().to_owned()))
    }

    /// Returns every key and value `table` holds in `range`, in key order.
    fn scan(
        &self,
        table: TableDefinition<'static, &str, &str>,
        range: &core::ops::Range<String>,
    ) -> Result<Vec<(String, String)>, StoreError> {
        let read = self.database.begin_read().map_err(transaction)?;
        let opened = read.open_table(table).map_err(transaction)?;
        let rows = opened
            .range(range.start.as_str()..range.end.as_str())
            .map_err(transaction)?;
        rows.map(|row| {
            let (key, value) = row.map_err(transaction)?;
            Ok((key.value().to_owned(), value.value().to_owned()))
        })
        .collect()
    }

    /// Writes `value` under `key` in `table` unless the key is already taken,
    /// and returns the value that stands.
    fn record(
        &self,
        table: TableDefinition<'static, &str, &str>,
        key: &str,
        value: &str,
    ) -> Result<String, StoreError> {
        let write = self.database.begin_write().map_err(transaction)?;
        let stood = {
            let mut opened = write.open_table(table).map_err(transaction)?;
            let existing = opened
                .get(key)
                .map_err(transaction)?
                .map(|held| held.value().to_owned());
            if let Some(held) = existing {
                held
            } else {
                opened.insert(key, value).map_err(transaction)?;
                value.to_owned()
            }
        };
        write.commit().map_err(transaction)?;
        Ok(stood)
    }
}

/// Returns the identifier this store read back, or a refusal naming the key.
fn identifier<T, E>(
    key: &str,
    kind: &'static str,
    value: Option<String>,
    parse: impl Fn(&str) -> Result<T, E>,
) -> Result<Option<T>, StoreError>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match value {
        None => Ok(None),
        Some(text) => match parse(&text) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(source) => Err(StoreError::Identifier {
                key: key.to_owned(),
                kind,
                value: text,
                source: Box::new(source),
            }),
        },
    }
}

/// Returns the record `text` holds, or a refusal naming the key.
fn record_of<T: serde::de::DeserializeOwned>(
    key: &str,
    text: Option<String>,
) -> Result<Option<T>, StoreError> {
    match text {
        None => Ok(None),
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| StoreError::Record {
                key: key.to_owned(),
                source: Box::new(source),
            }),
    }
}

/// Returns the record `value` writes, or a refusal naming the key.
fn rendered<T: serde::Serialize>(key: &str, value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|source| StoreError::Record {
        key: key.to_owned(),
        source: Box::new(source),
    })
}

/// Wraps a `redb` refusal as a store refusal that carries it.
fn transaction<E: Into<redb::Error>>(source: E) -> StoreError {
    StoreError::Transaction {
        source: Box::new(source.into()),
    }
}

impl Store for RedbStore {
    fn ehr_of(&self, patient: &PersonId) -> Result<Option<EhrId>, StoreError> {
        let key = patient.to_string();
        let held = self.get(PATIENTS, &key)?;
        identifier(&key, "ehr_id", held, EhrId::new)
    }

    fn record_ehr(&self, patient: &PersonId, ehr: &EhrId) -> Result<EhrId, StoreError> {
        let key = patient.to_string();
        let stood = self.record(PATIENTS, &key, ehr.as_str())?;
        identifier(&key, "ehr_id", Some(stood), EhrId::new)?.ok_or(StoreError::Missing { key })
    }

    fn internal_of(
        &self,
        resource_type: &str,
        external: &ExternalResourceId,
    ) -> Result<Option<FhirResourceId>, StoreError> {
        let key = external_key(resource_type, external);
        let held = self.get(EXTERNAL, &key)?;
        identifier(&key, "FHIR id", held, FhirResourceId::new)
    }

    fn record_internal(
        &self,
        resource_type: &str,
        external: &ExternalResourceId,
        internal: &FhirResourceId,
    ) -> Result<FhirResourceId, StoreError> {
        let key = external_key(resource_type, external);
        let stood = self.record(EXTERNAL, &key, internal.as_str())?;
        identifier(&key, "FHIR id", Some(stood), FhirResourceId::new)?
            .ok_or(StoreError::Missing { key })
    }

    fn binding_of(
        &self,
        resource_type: &str,
        internal: &FhirResourceId,
    ) -> Result<Option<CompositionBinding>, StoreError> {
        let key = internal_key(resource_type, internal);
        let held = self.get(BINDINGS, &key)?;
        record_of(&key, held)
    }

    fn record_binding(
        &self,
        resource_type: &str,
        internal: &FhirResourceId,
        binding: &CompositionBinding,
    ) -> Result<CompositionBinding, StoreError> {
        let key = internal_key(resource_type, internal);
        let offered = rendered(&key, binding)?;
        let stood = self.record(BINDINGS, &key, &offered)?;
        record_of(&key, Some(stood))?.ok_or(StoreError::Missing { key })
    }

    fn consumed(&self, source: &SourceVersion) -> Result<Option<ConsumedSource>, StoreError> {
        let key = source.storage_key();
        let held = self.get(SOURCES, &key)?;
        record_of(&key, held)
    }

    fn consumed_versions(&self, source: &SourceVersion) -> Result<Vec<ConsumedSource>, StoreError> {
        self.scan(SOURCES, &source.every_version())?
            .into_iter()
            .map(|(key, held)| record_of(&key, Some(held))?.ok_or(StoreError::Missing { key }))
            .collect()
    }

    fn record_consumed(
        &self,
        source: &SourceVersion,
        consumed: &ConsumedSource,
    ) -> Result<ConsumedSource, StoreError> {
        let key = source.storage_key();
        let offered = rendered(&key, consumed)?;
        let stood = self.record(SOURCES, &key, &offered)?;
        record_of(&key, Some(stood))?.ok_or(StoreError::Missing { key })
    }

    fn committed(&self, source: &SourceVersion) -> Result<Option<CommittedSource>, StoreError> {
        let key = source.storage_key();
        let held = self.get(CONTRIBUTIONS, &key)?;
        record_of(&key, held)
    }

    fn record_committed(
        &self,
        source: &SourceVersion,
        committed: &CommittedSource,
    ) -> Result<CommittedSource, StoreError> {
        let key = source.storage_key();
        let offered = rendered(&key, committed)?;
        let stood = self.record(CONTRIBUTIONS, &key, &offered)?;
        record_of(&key, Some(stood))?.ok_or(StoreError::Missing { key })
    }
}

#[cfg(test)]
mod tests {
    use super::RedbStore;
    use crate::cdr::ids::EhrId;
    use crate::facade::identity::record::{
        CommittedSource, CompositionBinding, ConsumedSource, SourceVersion,
    };
    use crate::facade::identity::store::Store;
    use crate::facade::identity::{ExternalResourceId, FhirResourceId, PersonId};

    /// A synthetic clinical value, to prove it never reaches the file.
    const MARKER: &str = "a synthetic finding the identity store must never hold";

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
    fn the_store_survives_a_restart() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("identity.redb");
        let internal = FhirResourceId::new("abc").expect("a legal FHIR id");
        {
            let store = RedbStore::open(&path).expect("the first open");
            store
                .record_binding("Condition", &internal, &binding("uid-1"))
                .expect("the write");
        }
        let reopened = RedbStore::open(&path).expect("the second open");
        let held = reopened
            .binding_of("Condition", &internal)
            .expect("the read")
            .expect("the binding survives");
        assert_eq!("uid-1", held.versioned_object_uid);
    }

    #[test]
    fn a_committed_source_survives_a_restart_and_is_recorded_once() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("identity.redb");
        let source = SourceVersion::new(
            "Condition",
            ExternalResourceId::new("sender-1").expect("a legal external id"),
            None,
        );
        let first = CommittedSource {
            ehr_id: String::from("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11"),
            contribution_uid: String::from("7b0a4c2e-0000-4000-8000-00000000000c"),
        };
        {
            let store = RedbStore::open(&path).expect("the first open");
            assert_eq!(None, store.committed(&source).expect("an empty read"));
            store
                .record_committed(&source, &first)
                .expect("the first write");
            let second = CommittedSource {
                contribution_uid: String::from("another"),
                ..first.clone()
            };
            assert_eq!(
                first,
                store
                    .record_committed(&source, &second)
                    .expect("the second write"),
                "a second write does not move the record"
            );
        }
        let reopened = RedbStore::open(&path).expect("the second open");
        assert_eq!(
            Some(first),
            reopened.committed(&source).expect("the read back")
        );
    }

    #[test]
    fn every_consumed_version_of_one_id_reads_back_from_disk_and_no_other_id() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = RedbStore::open(&directory.path().join("identity.redb")).expect("the open");
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
            ("c-1", Some("2"), "uid-1"),
            ("c-1", Some("1"), "uid-1"),
            ("c-10", None, "uid-3"),
        ] {
            store
                .record_consumed(&source(id, version), &consumed(uid))
                .expect("the write");
        }
        assert_eq!(
            vec![consumed("uid-1"), consumed("uid-1")],
            store
                .consumed_versions(&source("c-1", None))
                .expect("the read")
        );
        assert_eq!(
            Vec::<ConsumedSource>::new(),
            store
                .consumed_versions(&source("c-2", None))
                .expect("the read")
        );
    }

    #[test]
    fn the_map_wins_once_written_on_disk() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let store = RedbStore::open(&directory.path().join("identity.redb")).expect("the open");
        let internal = FhirResourceId::new("abc").expect("a legal FHIR id");
        store
            .record_binding("Condition", &internal, &binding("uid-1"))
            .expect("the first write");
        let second = store
            .record_binding("Condition", &internal, &binding("uid-2"))
            .expect("the second write");
        assert_eq!("uid-1", second.versioned_object_uid);
    }

    #[test]
    fn the_store_never_holds_clinical_content() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("identity.redb");
        let patient = PersonId::new("http://example.org/ns", "p-1").expect("a legal person id");
        let ehr = EhrId::new("bd6b1e5a-3b9b-4a4a-9e0b-9f4b3a0c9f11").expect("a legal ehr_id");
        let internal = FhirResourceId::new("abc").expect("a legal FHIR id");
        {
            let store = RedbStore::open(&path).expect("the open");
            store.record_ehr(&patient, &ehr).expect("the ehr write");
            store
                .record_binding("Condition", &internal, &binding("uid-1"))
                .expect("the binding write");
        }
        let bytes = std::fs::read(&path).expect("the store file reads back");
        assert!(
            !bytes
                .windows(MARKER.len())
                .any(|window| window == MARKER.as_bytes()),
            "the identity store file holds a clinical value"
        );
        assert!(
            bytes
                .windows(internal.as_str().len())
                .any(|window| window == internal.as_str().as_bytes()),
            "the identity store file does not hold the id it was given, so the marker search proves nothing"
        );
    }
}
