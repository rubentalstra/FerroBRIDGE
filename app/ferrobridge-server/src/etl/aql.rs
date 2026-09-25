// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The configured AQL, parsed and checked when the configuration loads.
//!
//! Each query names what the runner reads by its `AS` alias, so a result row
//! is read by position against the parsed `SELECT` list. A query that does
//! not parse as AQL 1.1.0 (the `openehr-query` grammar), lacks a projection
//! the runner reads, has no `ORDER BY`, or carries a `LIMIT` of its own is
//! refused before any call to the CDR. The runner pages with the `offset`
//! and `fetch` members of ITS-REST 1.1.0 (`query-codegen.openapi.yaml`,
//! `components.parameters.offset`), which name row numbers, and row numbers
//! only stay put across pages under a fixed order. No specification governs
//! the aliases or the ordering rule: our own design.

use openehr_query::lexer::{self, Token};
use openehr_query::parser::{self, ParseError};
use std::collections::BTreeMap;

/// The projections the composition query must alias, in no particular order.
pub const COMPOSITION_PROJECTIONS: [&str; 3] = ["ehr_id", "version_uid", "composition"];

/// The projections the composition query may alias.
///
/// A `versioned_object_uid` the query selects is checked against the
/// `version_uid`; one it does not select is read from the `version_uid`,
/// whose `object_id` part names the versioned object (openEHR RM Common 1.1.0,
/// `OBJECT_VERSION_ID`).
pub const OPTIONAL_COMPOSITION_PROJECTIONS: [&str; 1] = ["versioned_object_uid"];

/// The projections the visit query must alias, in no particular order.
pub const VISIT_PROJECTIONS: [&str; 4] = ["ehr_id", "visit_source", "visit_start", "visit_end"];

/// The query parameter `etl run --since` binds.
pub const SINCE_PARAMETER: &str = "since";

/// A configured query the runner refuses to send.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AqlError {
    /// The text is not AQL.
    #[error("the {query} does not parse as AQL")]
    Parse {
        /// Which query.
        query: &'static str,
        /// What the parser reported.
        #[source]
        source: ParseError,
    },
    /// A projection the runner reads is not aliased in the `SELECT` list.
    #[error("the {query} selects nothing `AS {projection}`, which the runner reads")]
    Missing {
        /// Which query.
        query: &'static str,
        /// The alias that is missing.
        projection: &'static str,
    },
    /// Two columns carry one alias.
    #[error("the {query} selects two columns `AS {alias}`")]
    Duplicate {
        /// Which query.
        query: &'static str,
        /// The alias.
        alias: String,
    },
    /// The query has no `ORDER BY`, so its pages have no fixed rows.
    #[error("the {query} has no ORDER BY, so paging it by offset could skip or repeat rows")]
    Unordered {
        /// Which query.
        query: &'static str,
    },
    /// The query limits its own rows, which the runner pages.
    #[error("the {query} carries a LIMIT; the runner pages the result itself")]
    Limited {
        /// Which query.
        query: &'static str,
    },
}

/// A query that parsed and carries every projection its reader needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedQuery {
    text: String,
    columns: BTreeMap<&'static str, usize>,
    parameters: Vec<String>,
}

impl CheckedQuery {
    /// Checks `text` as the composition query: `ehr_id`, `version_uid` and
    /// the whole `composition`, with `versioned_object_uid` when it selects
    /// one.
    ///
    /// # Errors
    ///
    /// Returns [`AqlError`] naming the first fault.
    pub fn compositions(text: &str) -> Result<Self, AqlError> {
        Self::check(
            "composition query",
            text,
            &COMPOSITION_PROJECTIONS,
            &OPTIONAL_COMPOSITION_PROJECTIONS,
        )
    }

    /// Checks `text` as the visit query: `ehr_id`, `visit_source`,
    /// `visit_start` and `visit_end`.
    ///
    /// # Errors
    ///
    /// Returns [`AqlError`] naming the first fault.
    pub fn visits(text: &str) -> Result<Self, AqlError> {
        Self::check("visit query", text, &VISIT_PROJECTIONS, &[])
    }

    /// Parses `text` and finds each of `projections`, and each of `optional`
    /// it selects, in its `SELECT` list.
    fn check(
        query: &'static str,
        text: &str,
        projections: &[&'static str],
        optional: &[&'static str],
    ) -> Result<Self, AqlError> {
        let parsed = parser::parse_str(text).map_err(|source| AqlError::Parse { query, source })?;
        if parsed.order_by.is_empty() {
            return Err(AqlError::Unordered { query });
        }
        if parsed.limit.is_some() {
            return Err(AqlError::Limited { query });
        }
        let mut aliases: BTreeMap<&str, usize> = BTreeMap::new();
        for (position, column) in parsed.select.columns.iter().enumerate() {
            if let Some(alias) = column.alias.as_deref()
                && aliases.insert(alias, position).is_some()
            {
                return Err(AqlError::Duplicate {
                    query,
                    alias: alias.to_owned(),
                });
            }
        }
        let mut columns = BTreeMap::new();
        for projection in projections {
            let position = aliases
                .get(projection)
                .copied()
                .ok_or(AqlError::Missing { query, projection })?;
            columns.insert(*projection, position);
        }
        for projection in optional {
            if let Some(position) = aliases.get(projection).copied() {
                columns.insert(*projection, position);
            }
        }
        let parameters = lexer::lex(text)
            .map_err(|source| AqlError::Parse {
                query,
                source: ParseError::Lex(source),
            })?
            .into_iter()
            .filter_map(|token| match token {
                Token::Parameter(name) => name.strip_prefix('$').map(str::to_owned),
                _ => None,
            })
            .collect();
        Ok(Self {
            text: text.to_owned(),
            columns,
            parameters,
        })
    }

    /// Returns the query text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the position of `projection` in a result row, when the query
    /// carries it.
    #[must_use]
    pub fn column(&self, projection: &str) -> Option<usize> {
        self.columns.get(projection).copied()
    }

    /// Returns whether the query names the parameter `$name`.
    #[must_use]
    pub fn names_parameter(&self, name: &str) -> bool {
        self.parameters.iter().any(|parameter| parameter == name)
    }
}

#[cfg(test)]
mod tests {
    use super::{AqlError, CheckedQuery, SINCE_PARAMETER};

    /// A composition query with the four projections.
    const COMPOSITIONS: &str = "SELECT e/ehr_id/value AS ehr_id, \
        v/uid/value AS version_uid, c AS composition, \
        vo/uid/value AS versioned_object_uid \
        FROM EHR e CONTAINS VERSIONED_OBJECT vo CONTAINS VERSION v CONTAINS COMPOSITION c \
        ORDER BY v/commit_audit/time_committed/value";

    #[test]
    fn the_four_projections_are_found_by_alias() {
        let query = CheckedQuery::compositions(COMPOSITIONS).expect("the query checks");
        assert_eq!(Some(0), query.column("ehr_id"));
        assert_eq!(Some(3), query.column("versioned_object_uid"));
        assert_eq!(Some(2), query.column("composition"));
        assert!(!query.names_parameter(SINCE_PARAMETER));
    }

    #[test]
    fn a_query_that_does_not_parse_is_refused() {
        let error = CheckedQuery::compositions("SELECT FROM WHERE").expect_err("not AQL");
        assert!(matches!(error, AqlError::Parse { .. }), "{error:?}");
    }

    #[test]
    fn a_missing_projection_is_refused_by_name() {
        let error = CheckedQuery::compositions(
            "SELECT e/ehr_id/value AS ehr_id, c AS composition FROM EHR e CONTAINS COMPOSITION c ORDER BY e/ehr_id/value",
        )
        .expect_err("the version_uid is missing");
        assert!(
            matches!(
                error,
                AqlError::Missing {
                    projection: "version_uid",
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_query_without_the_versioned_object_uid_checks_and_reads_none() {
        let query = CheckedQuery::compositions(
            "SELECT e/ehr_id/value AS ehr_id, v/uid/value AS version_uid, c AS composition \
             FROM EHR e CONTAINS VERSION v CONTAINS COMPOSITION c ORDER BY v/uid/value",
        )
        .expect("the query checks");
        assert_eq!(Some(1), query.column("version_uid"));
        assert_eq!(None, query.column("versioned_object_uid"));
    }
}
