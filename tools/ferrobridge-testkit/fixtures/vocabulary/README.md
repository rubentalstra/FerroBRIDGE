<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The synthetic OHDSI vocabulary

Ten CSV files, one per vocabulary table of the OMOP CDM v5.4
(<https://ohdsi.github.io/CommonDataModel/cdm54.html#Vocabulary_Tables>),
that the `omop-cdm` concept resolver tests load into a PostgreSQL built from
the vendored OHDSI DDL.

## Provenance

Every row was written by hand for the tests on 2026-09-25. No concept code,
concept name or vocabulary comes from an OHDSI vocabulary release or from any
licensed terminology. The vocabularies are `FB-SOURCE`, `FB-STANDARD`,
`FB-UNIT` and `FB-META`, each with the version `synthetic-1`. The domain
identifiers are the CDM's own names (`Measurement`, `Condition`, `Unit`,
`Drug`, `Metadata`) because the engine checks a resolved concept's domain
against the table a mapping writes, and the CDM field definitions name the
domains that way. The relationship identifiers `Maps to` and `Mapped from`
are the ones the CDM conventions name
(<https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>).

## The file shape is FerroBRIDGE's own

Each file is named after its table and carries a header row with the table's
columns in DDL order, then comma-separated rows in UTF-8, quoted where a value
holds a comma, with ISO 8601 dates and an unquoted empty field for NULL. That
is the shape PostgreSQL's `COPY ... FROM STDIN (FORMAT csv, HEADER MATCH)`
reads, and `ferrobridge_testkit::vocabulary::load` loads it that way.

It is not the Athena export format. That format has no public specification,
and #88 records it from an observed export. The production vocabulary loader
lands with #88 and reads that observed shape.

## What the rows cover

Each row isolates one rule of the resolver, so a few are shaped more simply
than a vocabulary release would ship them (a concept marked `U` with its
default end date, for example).

| Source key | What it exercises |
|---|---|
| `FB-STANDARD` `STD-MEAS` | a standard concept, returned as itself |
| `FB-SOURCE` `SRC-ONE` | one `Maps to` hop to one standard concept |
| `FB-SOURCE` `SRC-FAN` | one source concept mapped to two standard concepts, listed out of order |
| `FB-SOURCE` `SRC-DELETED` | a source concept with `invalid_reason` set |
| `FB-SOURCE` `SRC-STALE` | three mappings: a deleted relationship, an invalid target, one valid target |
| `FB-STANDARD` `STD-FUTURE`, `STD-PAST` | concepts whose validity dates exclude the record date |
| `FB-SOURCE` `SRC-LATER` | a relationship whose validity starts after the record date |
| `FB-SOURCE` `SRC-TWIN` | two valid concepts under one key |
| `FB-SOURCE` `SRC-SUCCEEDED` | two concepts under one key where only one is valid on the date |
| `FB-SOURCE` `SRC-NOMAP` | a valid source concept with no `Maps to` row |
| `FB-UNIT` `U-SYNTH` | a standard unit concept, with a multi-byte name |

The remaining tables carry a few rows each so the loader reads every one of
them: the `FB-META` concepts give the vocabularies, domains, classes and
relationships the concept their table references.
