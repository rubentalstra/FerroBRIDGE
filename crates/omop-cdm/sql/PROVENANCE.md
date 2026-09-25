<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->
<!-- The two SQL files beside this one are a modified form of OHDSI material and
     keep its licence, the Apache License 2.0. -->

# Provenance: the PostgreSQL form of the CDM era scripts

The OMOP CDM documentation publishes a "Condition Eras" and a "Drug Eras"
script on its SQL scripts page
(<https://ohdsi.github.io/CommonDataModel/sqlScripts.html>). The page source is
`site/sqlScripts.qmd` of `OHDSI/CommonDataModel`, which
`scripts/vendor/omop-cdm.sh` vendors verbatim at tag `v5.4.3` under
`docs/specs/omop-cdm/site/`. The repository declares the Apache License 2.0 in
its `DESCRIPTION` file (`docs/specs/omop-cdm/PROVENANCE.md`).

The published scripts are OHDSI SQL, the SQL Server dialect that SqlRender
translates, so PostgreSQL does not run them as written. The two files here are
that translation, made by hand, and differ from the published text only in
these ways:

| Published form | PostgreSQL form |
|---|---|
| `IF OBJECT_ID('tempdb..#t', 'U') IS NOT NULL DROP TABLE #t;` | `DROP TABLE IF EXISTS pg_temp.t;` |
| `SELECT ... INTO #t FROM ...` | `CREATE TEMP TABLE t AS SELECT ... FROM ...` |
| `#t` | `t` (a temporary table, found first on the search path) |
| `DATEADD(day, n, d)` | `(d + n)`, a `date` plus an `integer` |
| `DATEDIFF(day, a, b)` | `(b - a)`, the difference of two `date`s in days |
| `@TARGET_CDMV5_SCHEMA`, `@cdm_schema` | `@cdmDatabaseSchema`, the placeholder of OHDSI's rendered DDL |
| the SqlRender header and its `{DEFAULT ...}` and `USE` lines | left out |
| the `/* / */` separators and the `CONDITION ERA` banner comment | left out |

Every other line, the `--` comments included, is the published line. The
`omop-cdm` tests extract both scripts from the vendored page and compare their
digest with the one this translation was made from, so a change to either
published script fails the build until the translation follows it.

The page notes that the Condition Eras script "only works with 5.3 and below".
The CDM v5.4 `CONDITION_ERA` table has the six columns the script writes, with
the types it writes them in, and the `omop-cdm` tests run the PostgreSQL form
against a v5.4 schema built from the vendored DDL.

## Files

- `condition_era.sql`
- `drug_era.sql`
