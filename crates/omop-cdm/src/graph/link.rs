// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `FACT_RELATIONSHIP` between two rows of the graph.

use crate::graph::key::RecordKey;
use crate::graph::row::GraphError;
use crate::graph::row::cdm_table;
use crate::meta::TableMeta;

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
