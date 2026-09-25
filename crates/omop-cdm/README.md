<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# omop-cdm

The OMOP Common Data Model v5.4 for Rust: generated row types and column metadata, the embedded OHDSI DDL, the vocabulary loader and concept resolver.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

## What is here

- A row type per CDM table, all 39 across the `CDM`, `VOCAB` and `RESULTS`
  schemas, with every column in definition order. The types are generated from
  the OHDSI `CommonDataModel` field and table definitions at tag `v5.4.3`, so
  they say what the model says.
- The column metadata of every table: name, CDM datatype, Rust type, whether
  the column is required, the primary key, and the foreign key it follows.
- OHDSI's rendered PostgreSQL DDL, embedded verbatim, with the
  `@cdmDatabaseSchema` placeholder substituted by a schema name that is checked
  as an unquoted PostgreSQL identifier.
- The three column types Rust has no type for: `Varchar<N>` for a
  `varchar(n)`, and `CdmDate` and `CdmDatetime`, which keep the value as ISO
  8601 text and validate it at construction.

- `graph`: the record graph one openEHR composition becomes, the seam
  between a mapping engine and a CDM writer. A row is keyed by where it came
  from (the EHR, the versioned composition, the archetype root and the
  occurrence), checked against the column metadata as it is built, and names
  other rows by reference; the writer assigns the ids.

With the `database` feature, on by default:

- `database` and `vocabulary`: a PostgreSQL pool bound to one CDM schema, the
  DDL applied to it, and the concept resolver over the loaded vocabulary.
- `writer`: one record graph per transaction through binary `COPY`, with a
  natural-key side table that keeps each row on its id across runs and a
  watermark per composition.
- `derived`: `OBSERVATION_PERIOD`, and `CONDITION_ERA` and `DRUG_ERA` through
  the PostgreSQL form of the scripts the CDM publishes, under `sql/` with the
  Apache License 2.0 of their source (`sql/PROVENANCE.md`).

A consumer that reads only the metadata and the record graph depends on the
crate with `default-features = false` and builds no database client.

The loader for an Athena vocabulary export follows once its observed format is
recorded.

Version 0.0.0 reserves the crate name on crates.io; the crate line starts with
the first release of this content.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.

The files under `ddl/` are OHDSI's, copied verbatim under the Apache License
2.0 (`ddl/PROVENANCE.md`). The two SQL files under `sql/` are the PostgreSQL
form of OHDSI's era scripts and keep that licence (`sql/PROVENANCE.md`).
