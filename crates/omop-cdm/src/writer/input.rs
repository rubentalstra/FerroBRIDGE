// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a commit takes: the run, the person policy and the visit concepts.

use std::fmt;

use crate::graph::key::EmptyIdentifier;

/// The identifier of one ETL run, recorded with every watermark it writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

impl RunId {
    /// Wraps a run identifier, refusing an empty one.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyIdentifier`] when `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, EmptyIdentifier> {
        let value = value.into();
        if value.is_empty() {
            return Err(EmptyIdentifier::of("run id"));
        }
        Ok(Self(value))
    }

    /// Returns the identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the writer does for an EHR it has no `PERSON` for.
///
/// One `ehr_id` is one person; reconciling a person across EHRs is the
/// deployment's decision and never the writer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonPolicy {
    /// Assigns the EHR a `person_id` the first time a row refers to it.
    CreateOnFirstSight,
    /// Refuses a row that refers to an EHR no earlier `PERSON` row named.
    Existing,
}

/// The two concepts every derived visit carries, from configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisitConcepts {
    /// `visit_concept_id`.
    pub visit_concept_id: i32,
    /// `visit_type_concept_id`.
    pub visit_type_concept_id: i32,
}
