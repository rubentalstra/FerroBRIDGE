// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Concept resolution: a source code to its standard concepts, by SQL over
//! the vocabulary tables of a CDM schema.
//!
//! A [`SourceKey`] names a code in a vocabulary. [`ConceptResolver::resolve`]
//! finds the one valid `CONCEPT` row under that key on the record date and
//! returns it when it is standard, or the standard concepts its `Maps to`
//! relationships reach, every one of them. Each row the query touches must be
//! valid on the record date. The conventions are OHDSI's
//! (<https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>) and
//! the tables the CDM's
//! (<https://ohdsi.github.io/CommonDataModel/cdm54.html#Vocabulary_Tables>).
//!
//! The queries are checked at compile time against a database built from the
//! vendored DDL, through the `.sqlx/` metadata committed beside the crate.

#[cfg(feature = "database")]
use crate::database::CdmPool;
use crate::generated::concept::Concept;
use crate::value::{CdmDate, ValueError, Varchar};
use std::fmt;

/// A `concept_id`: the CDM's identifier of a concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConceptId(i32);

impl ConceptId {
    /// The concept the CDM writes where no standard concept exists.
    ///
    /// The CDM conventions: "If a Standard Concept does not exist or cannot
    /// be identified, the Concept with the `CONCEPT_ID` 0 is used".
    pub const NO_MATCHING_CONCEPT: Self = Self(0);

    /// Wraps a `concept_id`.
    #[must_use]
    pub const fn new(id: i32) -> Self {
        Self(id)
    }

    /// Returns the `concept_id`.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl From<&Concept> for ConceptId {
    fn from(concept: &Concept) -> Self {
        Self(concept.concept_id)
    }
}

impl fmt::Display for ConceptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A `vocabulary_id`: the vocabulary a concept code belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VocabularyId(Varchar<20>);

impl VocabularyId {
    /// Creates a vocabulary id, refusing one longer than the column's 20.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::TooLong`] when `id` exceeds `varchar(20)`.
    pub fn new(id: impl Into<String>) -> Result<Self, ValueError> {
        Varchar::new(id).map(Self)
    }

    /// Returns the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for VocabularyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A `concept_code`: a code as its source vocabulary writes it.
///
/// The CDM notes that concept codes are not unique across vocabularies, so a
/// code only names a concept together with a [`VocabularyId`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConceptCode(Varchar<50>);

impl ConceptCode {
    /// Creates a concept code, refusing one longer than the column's 50.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::TooLong`] when `code` exceeds `varchar(50)`.
    pub fn new(code: impl Into<String>) -> Result<Self, ValueError> {
        Varchar::new(code).map(Self)
    }

    /// Returns the code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ConceptCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The key a source code is looked up by: `(vocabulary_id, concept_code)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceKey {
    vocabulary: VocabularyId,
    code: ConceptCode,
}

impl SourceKey {
    /// Pairs a code with its vocabulary.
    #[must_use]
    pub fn new(vocabulary: VocabularyId, code: ConceptCode) -> Self {
        Self { vocabulary, code }
    }

    /// Returns the vocabulary.
    #[must_use]
    pub fn vocabulary(&self) -> &VocabularyId {
        &self.vocabulary
    }

    /// Returns the code.
    #[must_use]
    pub fn code(&self) -> &ConceptCode {
        &self.code
    }
}

impl fmt::Display for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "vocabulary_id '{}', concept_code '{}'",
            self.vocabulary, self.code
        )
    }
}

/// What a source code resolves to on a record date.
///
/// Every concept is a whole `CONCEPT` row, so its `domain_id` travels with it
/// for the caller to check against the table it writes.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// The key names a standard concept, which is its own resolution.
    Standard {
        /// The concept.
        concept: Concept,
    },
    /// The key names a non-standard concept whose `Maps to` relationships
    /// reach one or more standard concepts.
    Mapped {
        /// The concept the key names, the record's source concept.
        source: Concept,
        /// The standard concepts, ordered by `concept_id`; never empty.
        standard: Vec<Concept>,
    },
    /// No standard concept is reachable.
    ///
    /// The caller writes [`ConceptId::NO_MATCHING_CONCEPT`] and keeps the
    /// source value; counting the outcome is the caller's too.
    Unmapped {
        /// The key that was looked up.
        key: SourceKey,
        /// The valid non-standard concept the key names, when there is one.
        source: Option<Concept>,
    },
}

impl Resolution {
    /// Returns the standard concepts, empty for [`Resolution::Unmapped`].
    #[must_use]
    pub fn standard_concepts(&self) -> &[Concept] {
        match self {
            Self::Standard { concept } => std::slice::from_ref(concept),
            Self::Mapped { standard, .. } => standard,
            Self::Unmapped { .. } => &[],
        }
    }

    /// Returns the concept the key names, when a valid one exists.
    #[must_use]
    pub fn source_concept(&self) -> Option<&Concept> {
        match self {
            Self::Standard { concept } => Some(concept),
            Self::Mapped { source, .. } => Some(source),
            Self::Unmapped { source, .. } => source.as_ref(),
        }
    }
}

/// A lookup the resolver could not answer.
///
/// Every variant names the key and the record date it was looked up on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// The database refused or failed a query.
    #[cfg(feature = "database")]
    #[error("the vocabulary query for {key} on {date} failed")]
    Query {
        /// The key that was looked up.
        key: Box<SourceKey>,
        /// The record date.
        date: CdmDate,
        /// The database's error.
        #[source]
        source: Box<sqlx::Error>,
    },
    /// More than one valid concept carries the key, so the source concept is
    /// not determined.
    #[error(
        "{key} names {} valid concepts on {date} ({}), so the source concept is ambiguous",
        concept_ids.len(),
        join(concept_ids)
    )]
    Ambiguous {
        /// The key that was looked up.
        key: Box<SourceKey>,
        /// The record date.
        date: CdmDate,
        /// The concepts that carry it, ordered by `concept_id`.
        concept_ids: Vec<ConceptId>,
    },
    /// A vocabulary row holds a value its CDM column type refuses.
    #[error("concept {concept_id}, read for {key} on {date}, has a {column} the CDM refuses")]
    Row {
        /// The key that was looked up.
        key: Box<SourceKey>,
        /// The record date.
        date: CdmDate,
        /// The concept whose row is refused.
        concept_id: ConceptId,
        /// The column.
        column: &'static str,
        /// Why the value is refused.
        #[source]
        source: ValueError,
    },
}

/// A vocabulary lookup other than a source code that failed.
#[cfg(feature = "database")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LookupError {
    /// The database refused or failed the query.
    #[error("the vocabulary query for {what} failed")]
    Query {
        /// What was looked up.
        what: String,
        /// The database's error.
        #[source]
        source: Box<sqlx::Error>,
    },
    /// More than one concept answers the lookup.
    #[error("{what} names {} concepts ({})", concept_ids.len(), join(concept_ids))]
    Ambiguous {
        /// What was looked up.
        what: String,
        /// The concepts, ordered by `concept_id`.
        concept_ids: Vec<ConceptId>,
    },
}

/// Renders concept ids as a comma-separated list.
fn join(ids: &[ConceptId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolves source codes against the vocabulary tables of one CDM schema.
#[cfg(feature = "database")]
#[derive(Debug, Clone)]
pub struct ConceptResolver {
    pool: CdmPool,
}

#[cfg(feature = "database")]
impl ConceptResolver {
    /// Resolves against the vocabulary loaded in `pool`'s schema.
    #[must_use]
    pub fn new(pool: CdmPool) -> Self {
        Self { pool }
    }

    /// Returns the standard `Meas Value Operator` concept whose name is
    /// `symbol`, valid on `date`, `None` when the vocabulary holds none.
    ///
    /// The CDM names the operators `<`, `<=`, `=`, `>=` and `>` in that domain
    /// (`OMOP_CDMv5.4_Field_Level.csv`, `measurement.operator_concept_id`).
    ///
    /// # Errors
    ///
    /// Returns [`LookupError::Query`] when the database fails the query and
    /// [`LookupError::Ambiguous`] when more than one concept carries the name.
    pub async fn operator_concept(
        &self,
        symbol: &str,
        date: &CdmDate,
    ) -> Result<Option<ConceptId>, LookupError> {
        // NOTE: no specification governs this: our own design; the operator is
        // found by its concept name within the domain the CDM names.
        let ids = sqlx::query_scalar!(
            r#"SELECT concept_id FROM concept
               WHERE domain_id = 'Meas Value Operator' AND concept_name = $1
                 AND standard_concept = 'S' AND invalid_reason IS NULL
                 AND valid_start_date <= $2::text::date AND $2::text::date <= valid_end_date
               ORDER BY concept_id"#,
            symbol,
            date.as_str(),
        )
        .fetch_all(self.pool.pool())
        .await
        .map_err(|source| LookupError::Query {
            what: format!("the operator `{symbol}`"),
            source: Box::new(source),
        })?;
        match ids.as_slice() {
            [] => Ok(None),
            [only] => Ok(Some(ConceptId(*only))),
            several => Err(LookupError::Ambiguous {
                what: format!("the operator `{symbol}`"),
                concept_ids: several.iter().copied().map(ConceptId).collect(),
            }),
        }
    }

    /// Returns the `domain_concept_id` the `DOMAIN` table holds for `domain`,
    /// `None` when it holds no such domain.
    ///
    /// # Errors
    ///
    /// Returns [`LookupError::Query`] when the database fails the query.
    pub async fn domain_concept(&self, domain: &str) -> Result<Option<ConceptId>, LookupError> {
        let id = sqlx::query_scalar!(
            "SELECT domain_concept_id FROM domain WHERE domain_id = $1",
            domain,
        )
        .fetch_optional(self.pool.pool())
        .await
        .map_err(|source| LookupError::Query {
            what: format!("the domain `{domain}`"),
            source: Box::new(source),
        })?;
        Ok(id.map(ConceptId))
    }

    /// Resolves `key` as of the record date `date`.
    ///
    /// The concept the key names must be valid on `date`, and so must every
    /// `Maps to` relationship and every standard concept it reaches.
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError::Ambiguous`] when two or more valid concepts
    /// carry `key`, [`ResolveError::Query`] when the database fails a query,
    /// and [`ResolveError::Row`] when a row does not fit its CDM column.
    pub async fn resolve(
        &self,
        key: &SourceKey,
        date: &CdmDate,
    ) -> Result<Resolution, ResolveError> {
        let query_error = |source| ResolveError::Query {
            key: Box::new(key.clone()),
            date: date.clone(),
            source: Box::new(source),
        };
        let mut connection = self.pool.pool().acquire().await.map_err(query_error)?;

        // NOTE: CDM v5.4 CONCEPT.invalid_reason is NULL for a valid concept, and
        // valid_start_date and valid_end_date bound its validity.
        // NOTE: ordering by concept_id is our own design, for a deterministic result.
        let sources = sqlx::query_as!(
            ConceptRow,
            r#"SELECT concept_id, concept_name, domain_id, vocabulary_id, concept_class_id,
                      standard_concept, concept_code,
                      to_char(valid_start_date, 'YYYY-MM-DD') AS "valid_start_date!",
                      to_char(valid_end_date, 'YYYY-MM-DD') AS "valid_end_date!",
                      invalid_reason
               FROM concept
               WHERE vocabulary_id = $1 AND concept_code = $2
                 AND invalid_reason IS NULL
                 AND valid_start_date <= $3::text::date AND $3::text::date <= valid_end_date
               ORDER BY concept_id"#,
            key.vocabulary().as_str(),
            key.code().as_str(),
            date.as_str(),
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(query_error)?;

        let source = match sources.as_slice() {
            [] => {
                return Ok(Resolution::Unmapped {
                    key: key.clone(),
                    source: None,
                });
            }
            [row] => row.clone().into_concept(key, date)?,
            rows => {
                return Err(ResolveError::Ambiguous {
                    key: Box::new(key.clone()),
                    date: date.clone(),
                    concept_ids: rows.iter().map(|row| ConceptId(row.concept_id)).collect(),
                });
            }
        };

        // NOTE: CDM conventions, "Concepts": only a concept with standard_concept
        // 'S' may appear in a *_concept_id field.
        if source.standard_concept.as_ref().map(Varchar::as_str) == Some(STANDARD) {
            return Ok(Resolution::Standard { concept: source });
        }

        // NOTE: CDM conventions, "Mapping": one source concept may map to several
        // standard concepts, so every valid Maps to target is returned.
        let targets = sqlx::query_as!(
            ConceptRow,
            r#"SELECT DISTINCT c.concept_id, c.concept_name, c.domain_id, c.vocabulary_id,
                      c.concept_class_id, c.standard_concept, c.concept_code,
                      to_char(c.valid_start_date, 'YYYY-MM-DD') AS "valid_start_date!",
                      to_char(c.valid_end_date, 'YYYY-MM-DD') AS "valid_end_date!",
                      c.invalid_reason
               FROM concept_relationship AS r
               JOIN concept AS c ON c.concept_id = r.concept_id_2
               WHERE r.concept_id_1 = $1 AND r.relationship_id = 'Maps to'
                 AND r.invalid_reason IS NULL
                 AND r.valid_start_date <= $2::text::date AND $2::text::date <= r.valid_end_date
                 AND c.standard_concept = 'S'
                 AND c.invalid_reason IS NULL
                 AND c.valid_start_date <= $2::text::date AND $2::text::date <= c.valid_end_date
               ORDER BY c.concept_id"#,
            source.concept_id,
            date.as_str(),
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(query_error)?;

        if targets.is_empty() {
            return Ok(Resolution::Unmapped {
                key: key.clone(),
                source: Some(source),
            });
        }
        let standard = targets
            .into_iter()
            .map(|row| row.into_concept(key, date))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Resolution::Mapped { source, standard })
    }
}

/// The `standard_concept` flag of a standard concept.
#[cfg(feature = "database")]
const STANDARD: &str = "S";

#[cfg(feature = "database")]
/// One `CONCEPT` row as the queries select it, before the CDM column types
/// check it.
#[derive(Debug, Clone)]
struct ConceptRow {
    concept_id: i32,
    concept_name: String,
    domain_id: String,
    vocabulary_id: String,
    concept_class_id: String,
    standard_concept: Option<String>,
    concept_code: String,
    valid_start_date: String,
    valid_end_date: String,
    invalid_reason: Option<String>,
}

#[cfg(feature = "database")]
impl ConceptRow {
    /// Checks every column against its CDM type.
    fn into_concept(self, key: &SourceKey, date: &CdmDate) -> Result<Concept, ResolveError> {
        let concept_id = ConceptId(self.concept_id);
        let refused = |column| {
            move |source| ResolveError::Row {
                key: Box::new(key.clone()),
                date: date.clone(),
                concept_id,
                column,
                source,
            }
        };
        Ok(Concept {
            concept_id: self.concept_id,
            concept_name: Varchar::new(self.concept_name).map_err(refused("concept_name"))?,
            domain_id: Varchar::new(self.domain_id).map_err(refused("domain_id"))?,
            vocabulary_id: Varchar::new(self.vocabulary_id).map_err(refused("vocabulary_id"))?,
            concept_class_id: Varchar::new(self.concept_class_id)
                .map_err(refused("concept_class_id"))?,
            standard_concept: self
                .standard_concept
                .map(Varchar::new)
                .transpose()
                .map_err(refused("standard_concept"))?,
            concept_code: Varchar::new(self.concept_code).map_err(refused("concept_code"))?,
            valid_start_date: CdmDate::new(self.valid_start_date)
                .map_err(refused("valid_start_date"))?,
            valid_end_date: CdmDate::new(self.valid_end_date).map_err(refused("valid_end_date"))?,
            invalid_reason: self
                .invalid_reason
                .map(Varchar::new)
                .transpose()
                .map_err(refused("invalid_reason"))?,
        })
    }
}
