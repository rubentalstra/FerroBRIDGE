// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The synthetic OHDSI vocabulary and the loader that puts it into a CDM
//! schema.
//!
//! The ten files under `fixtures/vocabulary/` are invented for the tests,
//! one per vocabulary table of the CDM v5.4
//! (<https://ohdsi.github.io/CommonDataModel/cdm54.html#Vocabulary_Tables>).
//! Their shape is FerroBRIDGE's own, CSV with a header row that PostgreSQL's
//! `COPY` reads; it is not the Athena export format, which #88 records from an
//! observed export. `fixtures/vocabulary/README.md` lists what each row
//! exercises.
//!
//! No specification governs the file shape: our own design.

use sqlx::PgConnection;

/// One vocabulary table and the fixture rows it is loaded with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VocabularyTable {
    /// The table name, as the CDM DDL writes it.
    pub name: &'static str,
    /// The fixture: a header row with the table's columns in DDL order, then
    /// the rows.
    pub csv: &'static str,
}

/// The ten vocabulary tables, in the order the CDM documentation lists them.
pub const TABLES: [VocabularyTable; 10] = [
    VocabularyTable {
        name: "concept",
        csv: include_str!("../fixtures/vocabulary/concept.csv"),
    },
    VocabularyTable {
        name: "vocabulary",
        csv: include_str!("../fixtures/vocabulary/vocabulary.csv"),
    },
    VocabularyTable {
        name: "domain",
        csv: include_str!("../fixtures/vocabulary/domain.csv"),
    },
    VocabularyTable {
        name: "concept_class",
        csv: include_str!("../fixtures/vocabulary/concept_class.csv"),
    },
    VocabularyTable {
        name: "concept_relationship",
        csv: include_str!("../fixtures/vocabulary/concept_relationship.csv"),
    },
    VocabularyTable {
        name: "relationship",
        csv: include_str!("../fixtures/vocabulary/relationship.csv"),
    },
    VocabularyTable {
        name: "concept_synonym",
        csv: include_str!("../fixtures/vocabulary/concept_synonym.csv"),
    },
    VocabularyTable {
        name: "concept_ancestor",
        csv: include_str!("../fixtures/vocabulary/concept_ancestor.csv"),
    },
    VocabularyTable {
        name: "source_to_concept_map",
        csv: include_str!("../fixtures/vocabulary/source_to_concept_map.csv"),
    },
    VocabularyTable {
        name: "drug_strength",
        csv: include_str!("../fixtures/vocabulary/drug_strength.csv"),
    },
];

/// PostgreSQL refused a fixture file.
#[derive(Debug, thiserror::Error)]
#[error("PostgreSQL refused the synthetic {table} rows")]
pub struct LoadError {
    table: &'static str,
    #[source]
    source: sqlx::Error,
}

impl LoadError {
    /// Returns the table whose rows were refused.
    #[must_use]
    pub fn table(&self) -> &'static str {
        self.table
    }
}

/// Loads every [`TABLES`] fixture into the tables `connection` resolves by
/// its `search_path`, and returns the number of rows written.
///
/// Each file goes in through `COPY ... FROM STDIN (FORMAT csv, HEADER MATCH)`,
/// so a header that disagrees with the table's columns is refused
/// (<https://www.postgresql.org/docs/18/sql-copy.html>).
///
/// # Errors
///
/// Returns [`LoadError`] naming the first table PostgreSQL refuses.
pub async fn load(connection: &mut PgConnection) -> Result<u64, LoadError> {
    let mut rows = 0;
    for table in TABLES {
        let refused = |source| LoadError {
            table: table.name,
            source,
        };
        let statement = format!("COPY {} FROM STDIN (FORMAT csv, HEADER MATCH)", table.name);
        let mut copy = connection.copy_in_raw(&statement).await.map_err(refused)?;
        copy.send(table.csv.as_bytes()).await.map_err(refused)?;
        rows += copy.finish().await.map_err(refused)?;
    }
    Ok(rows)
}
