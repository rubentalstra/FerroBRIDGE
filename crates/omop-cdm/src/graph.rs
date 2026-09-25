// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The record graph: the CDM rows one composition becomes, keyed by where
//! they came from rather than by a surrogate id.
//!
//! The OMOCL engine emits a [`RecordGraph`] per composition and the CDM
//! writer commits it whole. Every CDM v5.4 primary key is a 32-bit
//! `integer`, so the ids are the writer's to assign: a [`Row`] carries a
//! [`RecordKey`] (the EHR, the versioned composition, the archetype root and
//! the occurrence) instead, and a column that names another row carries a
//! [`Reference`] the writer resolves. A [`Link`] is a `FACT_RELATIONSHIP`
//! between two rows of the graph. The [`Report`] carries the outcomes the
//! engine counts, and the writer completes it with what it wrote.
//!
//! A row is checked against the generated column metadata as it is built, so
//! a graph that exists names only real columns, holds only values their CDM
//! type admits, and carries every required column. No specification governs
//! this shape: our own design (the CDM leaves keys and provenance to the
//! ETL, <https://ohdsi.github.io/CommonDataModel/cdm54.html>).

use crate::meta::{CdmSchema, ColumnMeta, TableMeta};
use crate::value::{CdmDate, CdmDatetime};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

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
    /// The `ehr_id` of the EHR a composition belongs to.
    EhrId,
    "ehr_id"
);
identifier!(
    /// The `VERSIONED_OBJECT.uid` of a composition: the part of its version
    /// id that stays the same across its versions.
    VersionedObjectUid,
    "versioned object uid"
);
identifier!(
    /// The `VERSION.uid` of one composition version, an `OBJECT_VERSION_ID`.
    VersionUid,
    "version uid"
);
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    ehr_id: EhrId,
    versioned_object_uid: VersionedObjectUid,
    version_uid: VersionUid,
}

impl Source {
    /// Names a composition version.
    #[must_use]
    pub fn new(
        ehr_id: EhrId,
        versioned_object_uid: VersionedObjectUid,
        version_uid: VersionUid,
    ) -> Self {
        Self {
            ehr_id,
            versioned_object_uid,
            version_uid,
        }
    }

    /// Returns the EHR.
    #[must_use]
    pub fn ehr_id(&self) -> &EhrId {
        &self.ehr_id
    }

    /// Returns the versioned composition.
    #[must_use]
    pub fn versioned_object_uid(&self) -> &VersionedObjectUid {
        &self.versioned_object_uid
    }

    /// Returns the version.
    #[must_use]
    pub fn version_uid(&self) -> &VersionUid {
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
/// version's row replaces the earlier version's row under the same key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordKey {
    ehr_id: EhrId,
    versioned_object_uid: VersionedObjectUid,
    archetype_root_path: ArchetypeRootPath,
    occurrence_path: OccurrencePath,
    discriminator: Discriminator,
}

impl RecordKey {
    /// Names the row `discriminator` wrote from the node at
    /// `occurrence_path` under the archetype root at `archetype_root_path`
    /// of the versioned composition.
    #[must_use]
    pub fn new(
        ehr_id: EhrId,
        versioned_object_uid: VersionedObjectUid,
        archetype_root_path: ArchetypeRootPath,
        occurrence_path: OccurrencePath,
        discriminator: Discriminator,
    ) -> Self {
        Self {
            ehr_id,
            versioned_object_uid,
            archetype_root_path,
            occurrence_path,
            discriminator,
        }
    }

    /// Returns which mapping entry wrote the row.
    #[must_use]
    pub fn discriminator(&self) -> &Discriminator {
        &self.discriminator
    }

    /// Returns the EHR.
    #[must_use]
    pub fn ehr_id(&self) -> &EhrId {
        &self.ehr_id
    }

    /// Returns the versioned composition.
    #[must_use]
    pub fn versioned_object_uid(&self) -> &VersionedObjectUid {
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

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}{} {}",
            self.versioned_object_uid,
            self.archetype_root_path,
            self.occurrence_path,
            self.discriminator
        )
    }
}

/// The key of one visit: an EHR and the source its entries are grouped by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisitKey {
    ehr_id: EhrId,
    source: VisitSource,
}

impl VisitKey {
    /// Names the visit of `ehr_id` grouped under `source`.
    #[must_use]
    pub fn new(ehr_id: EhrId, source: VisitSource) -> Self {
        Self { ehr_id, source }
    }

    /// Returns the EHR.
    #[must_use]
    pub fn ehr_id(&self) -> &EhrId {
        &self.ehr_id
    }

    /// Returns the source.
    #[must_use]
    pub fn source(&self) -> &VisitSource {
        &self.source
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

/// A column value that names another row, resolved to its id by the writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// The `PERSON` of an EHR, for a column that references `person`.
    Person(EhrId),
    /// A derived visit, for a column that references `visit_occurrence`.
    Visit(VisitKey),
    /// Another row, in the table the column's foreign key names.
    Row(RecordKey),
}

/// A column value in the form its CDM datatype takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// An `integer` column.
    Integer(i32),
    /// A `float` column; never a NaN or an infinity.
    Float(f64),
    /// A `varchar(n)` or `varchar(max)` column.
    Text(String),
    /// A `date` column.
    Date(CdmDate),
    /// A `datetime` column.
    Datetime(CdmDatetime),
}

impl Value {
    /// Returns the kind of value, as the refusals name it.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Integer(_) => "integer",
            Self::Float(_) => "float",
            Self::Text(_) => "text",
            Self::Date(_) => "date",
            Self::Datetime(_) => "datetime",
        }
    }
}

/// What one column of a row holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// A value.
    Value(Value),
    /// A reference the writer resolves to an id.
    Reference(Reference),
}

/// A row or a link the graph refuses.
///
/// The variants name tables, columns and keys, never a value, because a
/// value of a clinical row is patient data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GraphError {
    /// The table is not a CDM table.
    #[error("`{table}` is not a table of the CDM schema")]
    UnknownTable {
        /// The name that was given.
        table: String,
    },
    /// The table cannot hold a mapped row.
    #[error("`{table}` takes no mapped rows: {why}")]
    NotWritable {
        /// The table.
        table: &'static str,
        /// Why.
        why: &'static str,
    },
    /// The column is not a column of the table.
    #[error("`{table}` has no column `{column}`")]
    UnknownColumn {
        /// The table.
        table: &'static str,
        /// The name that was given.
        column: String,
    },
    /// The column is the primary key, which the writer assigns.
    #[error("`{table}.{column}` is the primary key, which the writer assigns")]
    PrimaryKey {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The column was set twice.
    #[error("`{table}.{column}` is set twice")]
    Duplicate {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The value is not of the column's CDM datatype.
    #[error("`{table}.{column}` is `{datatype}` and was given a {given} value")]
    Mismatch {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The CDM datatype.
        datatype: &'static str,
        /// The kind of value given.
        given: &'static str,
    },
    /// The text is longer than the column's `varchar(n)`.
    #[error("`{table}.{column}` holds {limit} characters and was given {length}")]
    TooLong {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The column's bound.
        limit: usize,
        /// The length given, in characters.
        length: usize,
    },
    /// The float is a NaN or an infinity, which `NUMERIC` cannot hold.
    #[error("`{table}.{column}` was given a float that is not finite")]
    NotFinite {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The column cannot carry this reference.
    #[error("`{table}.{column}` cannot carry a {reference} reference")]
    Reference {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The kind of reference given.
        reference: &'static str,
    },
    /// A required column is not set.
    #[error("`{table}.{column}` is required and is not set")]
    Missing {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
    /// The row's key names another composition than the graph's.
    #[error("the row {key} of `{table}` belongs to another composition than the graph")]
    ForeignRow {
        /// The table.
        table: &'static str,
        /// The row's key.
        key: Box<RecordKey>,
    },
    /// Two rows of one table carry the same key.
    #[error("two rows of `{table}` carry the key {key}")]
    DuplicateRow {
        /// The table.
        table: &'static str,
        /// The key.
        key: Box<RecordKey>,
    },
    /// A link names a row the graph does not hold.
    #[error("a link names the row {key} of `{table}`, which the graph does not hold")]
    DanglingLink {
        /// The table.
        table: &'static str,
        /// The key.
        key: Box<RecordKey>,
    },
}

/// Returns the bound of a `varchar(n)` datatype, `None` for any other.
fn varchar_limit(datatype: &str) -> Option<usize> {
    // NOTE: no specification governs this: our own design; `varchar(MAX)` has
    // no number, so a parse failure is the answer "unbounded".
    datatype
        .strip_prefix("varchar(")?
        .strip_suffix(')')?
        .parse()
        .ok()
}

/// Returns the CDM-schema table with `name`.
///
/// # Errors
///
/// Returns [`GraphError::UnknownTable`] when no table of the `CDM` schema has
/// that name.
pub fn cdm_table(name: &str) -> Result<&'static TableMeta, GraphError> {
    crate::meta::table(name)
        .filter(|table| table.cdm_schema == CdmSchema::Cdm)
        .ok_or_else(|| GraphError::UnknownTable {
            table: name.to_owned(),
        })
}

/// The tables the derivations rebuild whole, which no mapped row may enter.
pub const DERIVED_TABLES: [&str; 4] = [
    "observation_period",
    "condition_era",
    "drug_era",
    "dose_era",
];

/// Returns the single integer primary key of `table`, when it has one.
#[must_use]
pub fn primary_key(table: &TableMeta) -> Option<&'static ColumnMeta> {
    let mut keys = table.columns.iter().filter(|column| column.primary_key);
    match (keys.next(), keys.next()) {
        (Some(key), None) if key.cdm_datatype == "integer" => Some(key),
        _ => None,
    }
}

/// One CDM row, keyed by its source, with every column checked against the
/// table's metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    table: &'static TableMeta,
    key: RecordKey,
    cells: BTreeMap<&'static str, Cell>,
}

impl Row {
    /// Starts a row of `table` under `key`, produced by the mapping its
    /// discriminator names.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownTable`] when `table` is not a CDM-schema
    /// table and [`GraphError::NotWritable`] for `fact_relationship` (a
    /// [`Link`] writes it), a derived table, or a table without a single
    /// integer primary key.
    pub fn builder(table: &str, key: RecordKey) -> Result<RowBuilder, GraphError> {
        let table = cdm_table(table)?;
        if table.name == "fact_relationship" {
            return Err(GraphError::NotWritable {
                table: table.name,
                why: "a Link writes it",
            });
        }
        if DERIVED_TABLES.contains(&table.name) {
            return Err(GraphError::NotWritable {
                table: table.name,
                why: "the derivations rebuild it whole",
            });
        }
        if primary_key(table).is_none() {
            return Err(GraphError::NotWritable {
                table: table.name,
                why: "it has no single integer primary key",
            });
        }
        Ok(RowBuilder {
            row: Self {
                table,
                key,
                cells: BTreeMap::new(),
            },
        })
    }

    /// Returns the table.
    #[must_use]
    pub fn table(&self) -> &'static TableMeta {
        self.table
    }

    /// Returns the natural key.
    #[must_use]
    pub fn key(&self) -> &RecordKey {
        &self.key
    }

    /// Returns the mapping that produced the row.
    #[must_use]
    pub fn mapping(&self) -> &MappingName {
        self.key.discriminator().mapping()
    }

    /// Returns the cells, by column name.
    #[must_use]
    pub fn cells(&self) -> &BTreeMap<&'static str, Cell> {
        &self.cells
    }

    /// Returns the cell of `column`, when it is set.
    #[must_use]
    pub fn cell(&self, column: &str) -> Option<&Cell> {
        self.cells.get(column)
    }
}

/// A [`Row`] being built; every setter checks the column first.
#[derive(Debug, Clone)]
pub struct RowBuilder {
    row: Row,
}

impl RowBuilder {
    /// Returns the column `name` of the row's table, refusing the primary key
    /// and a column set before.
    fn column(&self, name: &str) -> Result<&'static ColumnMeta, GraphError> {
        let table = self.row.table;
        let column = table
            .column(name)
            .ok_or_else(|| GraphError::UnknownColumn {
                table: table.name,
                column: name.to_owned(),
            })?;
        if column.primary_key {
            return Err(GraphError::PrimaryKey {
                table: table.name,
                column: column.name,
            });
        }
        if self.row.cells.contains_key(column.name) {
            return Err(GraphError::Duplicate {
                table: table.name,
                column: column.name,
            });
        }
        Ok(column)
    }

    /// Sets `column` to `value`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownColumn`], [`GraphError::PrimaryKey`] or
    /// [`GraphError::Duplicate`] for a column that cannot be set,
    /// [`GraphError::Mismatch`] when the value is not of the column's
    /// datatype, [`GraphError::TooLong`] for text beyond a `varchar(n)`, and
    /// [`GraphError::NotFinite`] for a NaN or an infinity.
    pub fn value(mut self, column: &str, value: Value) -> Result<Self, GraphError> {
        let meta = self.column(column)?;
        let table = self.row.table.name;
        let mismatch = || GraphError::Mismatch {
            table,
            column: meta.name,
            datatype: meta.cdm_datatype,
            given: value.kind(),
        };
        match (&value, meta.cdm_datatype) {
            (Value::Integer(_), "integer")
            | (Value::Date(_), "date")
            | (Value::Datetime(_), "datetime") => {}
            (Value::Float(number), "float") => {
                if !number.is_finite() {
                    return Err(GraphError::NotFinite {
                        table,
                        column: meta.name,
                    });
                }
            }
            (Value::Text(text), datatype) if datatype.starts_with("varchar(") => {
                if let Some(limit) = varchar_limit(datatype) {
                    let length = text.chars().count();
                    if length > limit {
                        return Err(GraphError::TooLong {
                            table,
                            column: meta.name,
                            limit,
                            length,
                        });
                    }
                } else if datatype != "varchar(MAX)" {
                    return Err(mismatch());
                }
            }
            _ => return Err(mismatch()),
        }
        self.row.cells.insert(meta.name, Cell::Value(value));
        Ok(self)
    }

    /// Sets `column` to `value` when there is one, and leaves it unset
    /// otherwise.
    ///
    /// # Errors
    ///
    /// Returns what [`RowBuilder::value`] returns.
    pub fn optional(self, column: &str, value: Option<Value>) -> Result<Self, GraphError> {
        match value {
            Some(value) => self.value(column, value),
            None => Ok(self),
        }
    }

    /// Sets `column` to a reference the writer resolves.
    ///
    /// A [`Reference::Person`] fits a column whose foreign key is
    /// `person.person_id`, a [`Reference::Visit`] one whose foreign key is
    /// `visit_occurrence.visit_occurrence_id`, and a [`Reference::Row`] one
    /// whose foreign key names any other CDM table with an integer key.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownColumn`], [`GraphError::PrimaryKey`] or
    /// [`GraphError::Duplicate`] for a column that cannot be set, and
    /// [`GraphError::Reference`] when the column's foreign key does not fit
    /// the reference.
    pub fn reference(mut self, column: &str, reference: Reference) -> Result<Self, GraphError> {
        let meta = self.column(column)?;
        let target = meta.foreign_key.map(|(table, _)| table);
        let fits = match (&reference, target) {
            (Reference::Person(_), Some("person"))
            | (Reference::Visit(_), Some("visit_occurrence")) => true,
            (Reference::Row(_), Some(table)) => {
                !matches!(table, "person" | "visit_occurrence")
                    && crate::meta::table(table)
                        .filter(|target| target.cdm_schema == CdmSchema::Cdm)
                        .and_then(primary_key)
                        .is_some()
            }
            _ => false,
        };
        if !fits {
            return Err(GraphError::Reference {
                table: self.row.table.name,
                column: meta.name,
                reference: match reference {
                    Reference::Person(_) => "person",
                    Reference::Visit(_) => "visit",
                    Reference::Row(_) => "row",
                },
            });
        }
        self.row.cells.insert(meta.name, Cell::Reference(reference));
        Ok(self)
    }

    /// Finishes the row.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Missing`] naming the first required column, in
    /// definition order, that is not set. A required date is never filled in.
    pub fn build(self) -> Result<Row, GraphError> {
        let table = self.row.table;
        for column in table.columns {
            if column.required && !column.primary_key && !self.row.cells.contains_key(column.name) {
                return Err(GraphError::Missing {
                    table: table.name,
                    column: column.name,
                });
            }
        }
        Ok(self.row)
    }
}

/// One end of a [`Link`]: a row of the graph and the concept of its domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEnd {
    table: &'static TableMeta,
    key: RecordKey,
    domain_concept_id: i32,
}

impl LinkEnd {
    /// Names the row of `table` under `key`, whose domain is the concept
    /// `domain_concept_id`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::UnknownTable`] when `table` is not a CDM-schema
    /// table.
    pub fn new(table: &str, key: RecordKey, domain_concept_id: i32) -> Result<Self, GraphError> {
        Ok(Self {
            table: cdm_table(table)?,
            key,
            domain_concept_id,
        })
    }

    /// Returns the table.
    #[must_use]
    pub fn table(&self) -> &'static TableMeta {
        self.table
    }

    /// Returns the row's key.
    #[must_use]
    pub fn key(&self) -> &RecordKey {
        &self.key
    }

    /// Returns the `domain_concept_id` the link writes for this end.
    #[must_use]
    pub fn domain_concept_id(&self) -> i32 {
        self.domain_concept_id
    }
}

/// A `FACT_RELATIONSHIP` between two rows of one graph.
///
/// The writer writes it in both directions with `relationship_concept_id`
/// 0: the CDM asks for a fact relationship in each direction
/// (<https://ohdsi.github.io/CommonDataModel/cdm54.html#fact_relationship>),
/// and no concept exists for the relationships the OMOCL library links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    first: LinkEnd,
    second: LinkEnd,
}

impl Link {
    /// The `relationship_concept_id` every link carries.
    pub const RELATIONSHIP_CONCEPT_ID: i32 = 0;

    /// Links `first` and `second`.
    #[must_use]
    pub fn new(first: LinkEnd, second: LinkEnd) -> Self {
        Self { first, second }
    }

    /// Returns the first end.
    #[must_use]
    pub fn first(&self) -> &LinkEnd {
        &self.first
    }

    /// Returns the second end.
    #[must_use]
    pub fn second(&self) -> &LinkEnd {
        &self.second
    }
}

/// A record the engine or the writer refused, and why.
///
/// The fields name where the record came from and never carry a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    mapping: Option<MappingName>,
    table: Option<&'static str>,
    column: Option<&'static str>,
    element: Option<String>,
    reason: String,
}

impl Refusal {
    /// Records a refusal for `reason`.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            mapping: None,
            table: None,
            column: None,
            element: None,
            reason: reason.into(),
        }
    }

    /// Records a refusal of a row the graph refused to build.
    #[must_use]
    pub fn of_row(error: &GraphError) -> Self {
        let (table, column) = match error {
            GraphError::NotWritable { table, .. }
            | GraphError::ForeignRow { table, .. }
            | GraphError::DuplicateRow { table, .. }
            | GraphError::DanglingLink { table, .. } => (Some(*table), None),
            GraphError::PrimaryKey { table, column }
            | GraphError::Duplicate { table, column }
            | GraphError::Mismatch { table, column, .. }
            | GraphError::TooLong { table, column, .. }
            | GraphError::NotFinite { table, column }
            | GraphError::Reference { table, column, .. }
            | GraphError::Missing { table, column } => (Some(*table), Some(*column)),
            GraphError::UnknownTable { .. } | GraphError::UnknownColumn { .. } => (None, None),
        };
        Self {
            mapping: None,
            table,
            column,
            element: None,
            reason: error.to_string(),
        }
    }

    /// Names the mapping the record came from.
    #[must_use]
    pub fn with_mapping(mut self, mapping: MappingName) -> Self {
        self.mapping = Some(mapping);
        self
    }

    /// Names the CDM table the refused record would have written.
    #[must_use]
    pub fn with_table(mut self, table: &'static str) -> Self {
        self.table = Some(table);
        self
    }

    /// Names the column the refusal is about: an OMOCL key or a CDM column.
    #[must_use]
    pub fn with_column(mut self, column: &'static str) -> Self {
        self.column = Some(column);
        self
    }

    /// Names the openEHR element, as an RM path, the refusal is about.
    #[must_use]
    pub fn with_element(mut self, element: impl Into<String>) -> Self {
        self.element = Some(element.into());
        self
    }

    /// Returns the mapping.
    #[must_use]
    pub fn mapping(&self) -> Option<&MappingName> {
        self.mapping.as_ref()
    }

    /// Returns the table.
    #[must_use]
    pub fn table(&self) -> Option<&'static str> {
        self.table
    }

    /// Returns the column.
    #[must_use]
    pub fn column(&self) -> Option<&'static str> {
        self.column
    }

    /// Returns the openEHR element.
    #[must_use]
    pub fn element(&self) -> Option<&str> {
        self.element.as_deref()
    }

    /// Returns why the record was refused.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// A source field no mapping wrote anywhere.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnmappedField {
    mapping: MappingName,
    element: String,
}

impl UnmappedField {
    /// Records that `mapping` wrote the element at the RM path `element`
    /// nowhere.
    #[must_use]
    pub fn new(mapping: MappingName, element: impl Into<String>) -> Self {
        Self {
            mapping,
            element: element.into(),
        }
    }

    /// Returns the mapping.
    #[must_use]
    pub fn mapping(&self) -> &MappingName {
        &self.mapping
    }

    /// Returns the element's RM path.
    #[must_use]
    pub fn element(&self) -> &str {
        &self.element
    }
}

/// The counted outcomes of one composition.
///
/// The engine records the refusals and the unmapped fields as it maps; the
/// writer records what it wrote with [`Report::record_written`]. Every
/// counter is keyed, so a run report can sum them per mapping and in total.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    rows: BTreeMap<(MappingName, &'static str), u64>,
    concept_zero: BTreeMap<(MappingName, &'static str, &'static str), u64>,
    zero_relationship_links: u64,
    unmapped: Vec<UnmappedField>,
    refusals: Vec<Refusal>,
}

impl Report {
    /// Records a refused record.
    pub fn refuse(&mut self, refusal: Refusal) {
        self.refusals.push(refusal);
    }

    /// Records a source field no mapping wrote.
    pub fn unmapped(&mut self, field: UnmappedField) {
        self.unmapped.push(field);
    }

    /// Records that the writer wrote `rows` of `table` for `mapping`, with
    /// `concept_zero` of them holding concept 0 in each named column.
    pub fn record_written(
        &mut self,
        mapping: &MappingName,
        table: &'static str,
        rows: u64,
        concept_zero: &BTreeMap<&'static str, u64>,
    ) {
        let entry = self.rows.entry((mapping.clone(), table)).or_default();
        *entry = entry.saturating_add(rows);
        for (column, count) in concept_zero {
            let entry = self
                .concept_zero
                .entry((mapping.clone(), table, column))
                .or_default();
            *entry = entry.saturating_add(*count);
        }
    }

    /// Records that the writer wrote `rows` `FACT_RELATIONSHIP` rows with
    /// `relationship_concept_id` 0.
    pub fn record_links(&mut self, rows: u64) {
        self.zero_relationship_links = self.zero_relationship_links.saturating_add(rows);
    }

    /// Returns the rows written, by mapping and table.
    #[must_use]
    pub fn rows(&self) -> &BTreeMap<(MappingName, &'static str), u64> {
        &self.rows
    }

    /// Returns the concept-0 assignments, by mapping, table and column.
    #[must_use]
    pub fn concept_zero(&self) -> &BTreeMap<(MappingName, &'static str, &'static str), u64> {
        &self.concept_zero
    }

    /// Returns the `FACT_RELATIONSHIP` rows written with
    /// `relationship_concept_id` 0.
    #[must_use]
    pub fn zero_relationship_links(&self) -> u64 {
        self.zero_relationship_links
    }

    /// Returns the source fields no mapping wrote.
    #[must_use]
    pub fn unmapped_fields(&self) -> &[UnmappedField] {
        &self.unmapped
    }

    /// Returns the refused records.
    #[must_use]
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }
}

/// The rows and links one composition version becomes.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordGraph {
    source: Source,
    rows: Vec<Row>,
    links: Vec<Link>,
    keys: BTreeSet<(&'static str, RecordKey)>,
    report: Report,
}

impl RecordGraph {
    /// Starts the empty graph of `source`.
    #[must_use]
    pub fn new(source: Source) -> Self {
        Self {
            source,
            rows: Vec::new(),
            links: Vec::new(),
            keys: BTreeSet::new(),
            report: Report::default(),
        }
    }

    /// Adds `row`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::ForeignRow`] when the row's key names another
    /// EHR or versioned composition than the graph's source, and
    /// [`GraphError::DuplicateRow`] when the table already holds a row under
    /// the key.
    pub fn push_row(&mut self, row: Row) -> Result<(), GraphError> {
        let key = row.key();
        if key.ehr_id() != self.source.ehr_id()
            || key.versioned_object_uid() != self.source.versioned_object_uid()
        {
            return Err(GraphError::ForeignRow {
                table: row.table().name,
                key: Box::new(key.clone()),
            });
        }
        if !self.keys.insert((row.table().name, key.clone())) {
            return Err(GraphError::DuplicateRow {
                table: row.table().name,
                key: Box::new(key.clone()),
            });
        }
        self.rows.push(row);
        Ok(())
    }

    /// Adds `link`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::DanglingLink`] when an end names a row the graph
    /// does not hold; push the rows first.
    pub fn push_link(&mut self, link: Link) -> Result<(), GraphError> {
        for end in [link.first(), link.second()] {
            if !self.keys.contains(&(end.table().name, end.key().clone())) {
                return Err(GraphError::DanglingLink {
                    table: end.table().name,
                    key: Box::new(end.key().clone()),
                });
            }
        }
        self.links.push(link);
        Ok(())
    }

    /// Returns the composition version the graph was mapped from.
    #[must_use]
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Returns the rows, in the order they were added.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Returns the links, in the order they were added.
    #[must_use]
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Returns the report.
    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Returns the report, for the engine to record into.
    pub fn report_mut(&mut self) -> &mut Report {
        &mut self.report
    }

    /// Returns the report, consuming the graph.
    #[must_use]
    pub fn into_report(self) -> Report {
        self.report
    }
}
