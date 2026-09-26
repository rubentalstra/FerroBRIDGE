// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identifiers a row is keyed by: where it came from, which occurrence
//! it is, and the visit it belongs to.

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

use crate::graph::row::GraphError;
use crate::graph::row::cdm_table;
use crate::graph::varchar_limit;
use crate::value::CdmDate;
use crate::value::CdmDatetime;

/// An identifier the graph refuses because it is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the {kind} is empty")]
pub struct EmptyIdentifier {
    kind: &'static str,
}

impl EmptyIdentifier {
    /// Refuses an empty identifier of `kind`.
    #[cfg(feature = "database")]
    pub(crate) fn of(kind: &'static str) -> Self {
        Self { kind }
    }

    /// Returns what the identifier names.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        self.kind
    }
}

/// Declares a non-empty text identifier.
macro_rules! identifier {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Wraps a ", $kind, ", refusing an empty one.")]
            ///
            /// # Errors
            ///
            /// Returns [`EmptyIdentifier`] when `value` is empty.
            pub fn new(value: impl Into<String>) -> Result<Self, EmptyIdentifier> {
                let value = value.into();
                if value.is_empty() {
                    return Err(EmptyIdentifier { kind: $kind });
                }
                Ok(Self(value))
            }

            /// Returns the identifier as text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

identifier!(
    /// The RM path of the archetype root a row was mapped from.
    ArchetypeRootPath,
    "archetype root path"
);
identifier!(
    /// The RM path, with its occurrence indices, of the node a row was
    /// mapped from.
    OccurrencePath,
    "occurrence path"
);
identifier!(
    /// The `metadata.name` of the mapping that produced a row.
    MappingName,
    "mapping name"
);
identifier!(
    /// The source a visit is grouped by within one EHR, written to
    /// `visit_source_value`.
    VisitSource,
    "visit source"
);

/// The composition version a graph was mapped from.
///
/// The EHR is RM EHR §`EHR.ehr_id` and the version RM Common §`VERSION.uid`,
/// carried as the `openehr-base` BASE 1.3 types the CDR's identifiers parse
/// into, so an id flows from the AQL row to the side table without a second
/// parse.
// NOTE: RM EHR §`ehr_id` is a HIER_OBJECT_ID; the ITS-REST client's EhrId would
// pull its HTTP stack into a metadata-only consumer of this crate.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    ehr_id: HierObjectId,
    versioned_object_uid: HierObjectId,
    version_uid: ObjectVersionId,
}

impl Source {
    /// Names the version `version_uid` of a composition in the EHR `ehr_id`.
    ///
    /// The versioned composition is the `object_id` part of the version's
    /// identifier (RM Common §`OBJECT_VERSION_ID`), taken as written: the
    /// typed `Uid` accessor lower-cases a UUID, and the side table keys a
    /// composition by the spelling the CDR issued.
    ///
    /// # Panics
    ///
    /// Never: `ObjectVersionId::new` refuses a value whose `object_id` part
    /// is not a legal `uid`, which is all `HierObjectId::new` checks.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "ObjectVersionId::new checked the object_id part against the uid production HierObjectId::new checks"
    )]
    pub fn new(ehr_id: HierObjectId, version_uid: ObjectVersionId) -> Self {
        let object_id = version_uid
            .value()
            .split_once("::")
            .map_or(version_uid.value(), |(head, _)| head);
        let versioned_object_uid = HierObjectId::new(object_id)
            .expect("the object_id of a constructed OBJECT_VERSION_ID should be a legal uid");
        Self {
            ehr_id,
            versioned_object_uid,
            version_uid,
        }
    }

    /// Returns the EHR.
    #[must_use]
    pub fn ehr_id(&self) -> &HierObjectId {
        &self.ehr_id
    }

    /// Returns the versioned composition.
    #[must_use]
    pub fn versioned_object_uid(&self) -> &HierObjectId {
        &self.versioned_object_uid
    }

    /// Returns the version.
    #[must_use]
    pub fn version_uid(&self) -> &ObjectVersionId {
        &self.version_uid
    }
}

/// Which mapping entry wrote a row, and which of its rows it is.
///
/// One node can feed several rows of one table: two entries of one mapping at
/// one archetype root, or one source code the vocabulary maps to several
/// standard concepts. The mapping, the entry index and the branch tell them
/// apart. No specification governs this: our own design.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Discriminator {
    mapping: MappingName,
    entry: u16,
    branch: u16,
}

impl Discriminator {
    /// Names the entry at index `entry` of `mapping`, and its `branch`th row
    /// (`0` for the one row of an entry that writes one).
    #[must_use]
    pub fn new(mapping: MappingName, entry: u16, branch: u16) -> Self {
        Self {
            mapping,
            entry,
            branch,
        }
    }

    /// Returns the mapping.
    #[must_use]
    pub fn mapping(&self) -> &MappingName {
        &self.mapping
    }

    /// Returns the index of the entry within the mapping.
    #[must_use]
    pub fn entry(&self) -> u16 {
        self.entry
    }

    /// Returns which of the entry's rows this is.
    #[must_use]
    pub fn branch(&self) -> u16 {
        self.branch
    }
}

impl fmt::Display for Discriminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}.{}", self.mapping, self.entry, self.branch)
    }
}

/// The natural key of one row: where in which composition it came from, and
/// which mapping entry wrote it.
///
/// The key stays the same across the versions of a composition, so a later
/// version's row replaces the earlier version's row under the same key. Two
/// keys compare by their identifiers as written, the spelling the side table
/// stores.
#[derive(Debug, Clone)]
pub struct RecordKey {
    ehr_id: HierObjectId,
    versioned_object_uid: HierObjectId,
    archetype_root_path: ArchetypeRootPath,
    occurrence_path: OccurrencePath,
    discriminator: Discriminator,
}

impl RecordKey {
    /// Names the row `discriminator` wrote from the node at
    /// `occurrence_path` under the archetype root at `archetype_root_path`
    /// of the EHR and versioned composition `source` names.
    #[must_use]
    pub fn new(
        source: &Source,
        archetype_root_path: ArchetypeRootPath,
        occurrence_path: OccurrencePath,
        discriminator: Discriminator,
    ) -> Self {
        Self {
            ehr_id: source.ehr_id.clone(),
            versioned_object_uid: source.versioned_object_uid.clone(),
            archetype_root_path,
            occurrence_path,
            discriminator,
        }
    }

    /// Returns the fields in the order the key sorts by.
    fn parts(
        &self,
    ) -> (
        &str,
        &str,
        &ArchetypeRootPath,
        &OccurrencePath,
        &Discriminator,
    ) {
        (
            self.ehr_id.value(),
            self.versioned_object_uid.value(),
            &self.archetype_root_path,
            &self.occurrence_path,
            &self.discriminator,
        )
    }

    /// Returns which mapping entry wrote the row.
    #[must_use]
    pub fn discriminator(&self) -> &Discriminator {
        &self.discriminator
    }

    /// Returns the EHR.
    #[must_use]
    pub fn ehr_id(&self) -> &HierObjectId {
        &self.ehr_id
    }

    /// Returns the versioned composition.
    #[must_use]
    pub fn versioned_object_uid(&self) -> &HierObjectId {
        &self.versioned_object_uid
    }

    /// Returns the archetype root path.
    #[must_use]
    pub fn archetype_root_path(&self) -> &ArchetypeRootPath {
        &self.archetype_root_path
    }

    /// Returns the occurrence path.
    #[must_use]
    pub fn occurrence_path(&self) -> &OccurrencePath {
        &self.occurrence_path
    }
}

impl PartialEq for RecordKey {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for RecordKey {}

impl PartialOrd for RecordKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RecordKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts().cmp(&other.parts())
    }
}

impl Hash for RecordKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parts().hash(state);
    }
}

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}{} {}",
            self.versioned_object_uid.value(),
            self.archetype_root_path,
            self.occurrence_path,
            self.discriminator
        )
    }
}

/// The key of one visit: an EHR and the source its entries are grouped by.
///
/// Two keys compare by the EHR identifier as written, the spelling the side
/// table stores.
#[derive(Debug, Clone)]
pub struct VisitKey {
    ehr_id: HierObjectId,
    source: VisitSource,
}

impl VisitKey {
    /// Names the visit of `ehr_id` grouped under `source`.
    #[must_use]
    pub fn new(ehr_id: HierObjectId, source: VisitSource) -> Self {
        Self { ehr_id, source }
    }

    /// Returns the fields in the order the key sorts by.
    fn parts(&self) -> (&str, &VisitSource) {
        (self.ehr_id.value(), &self.source)
    }

    /// Returns the EHR.
    #[must_use]
    pub fn ehr_id(&self) -> &HierObjectId {
        &self.ehr_id
    }

    /// Returns the source.
    #[must_use]
    pub fn source(&self) -> &VisitSource {
        &self.source
    }
}

impl PartialEq for VisitKey {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for VisitKey {}

impl PartialOrd for VisitKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VisitKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts().cmp(&other.parts())
    }
}

impl Hash for VisitKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parts().hash(state);
    }
}

/// One derived `VISIT_OCCURRENCE`: a visit key and the dates it spans.
///
/// The CDM requires both dates, so a visit is never built without them; the
/// datetimes are optional and written when the source carries a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    key: VisitKey,
    start: CdmDate,
    start_datetime: Option<CdmDatetime>,
    end: CdmDate,
    end_datetime: Option<CdmDatetime>,
}

impl Visit {
    /// Describes the visit under `key` from `start` to `end`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::TooLong`] when the key's source does not fit
    /// `visit_occurrence.visit_source_value`, which carries it.
    pub fn new(
        key: VisitKey,
        start: (CdmDate, Option<CdmDatetime>),
        end: (CdmDate, Option<CdmDatetime>),
    ) -> Result<Self, GraphError> {
        let table = cdm_table("visit_occurrence")?;
        if let Some(column) = table.column("visit_source_value")
            && let Some(limit) = varchar_limit(column.cdm_datatype)
        {
            let length = key.source().as_str().chars().count();
            if length > limit {
                return Err(GraphError::TooLong {
                    table: table.name,
                    column: column.name,
                    limit,
                    length,
                });
            }
        }
        Ok(Self {
            key,
            start: start.0,
            start_datetime: start.1,
            end: end.0,
            end_datetime: end.1,
        })
    }

    /// Returns the key.
    #[must_use]
    pub fn key(&self) -> &VisitKey {
        &self.key
    }

    /// Returns the start date.
    #[must_use]
    pub fn start(&self) -> &CdmDate {
        &self.start
    }

    /// Returns the start datetime, when the source carries a time.
    #[must_use]
    pub fn start_datetime(&self) -> Option<&CdmDatetime> {
        self.start_datetime.as_ref()
    }

    /// Returns the end date.
    #[must_use]
    pub fn end(&self) -> &CdmDate {
        &self.end
    }

    /// Returns the end datetime, when the source carries a time.
    #[must_use]
    pub fn end_datetime(&self) -> Option<&CdmDatetime> {
        self.end_datetime.as_ref()
    }
}
