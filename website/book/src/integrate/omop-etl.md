<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The OMOP ETL

The OMOP side of FerroBRIDGE turns openEHR content into rows in an OMOP Common
Data Model v5.4 database, driven by OMOCL mapping files. OMOP is a relational
analytics schema rather than an API, so the deliverable is populated tables.
The sink, the runner and `ferrobridge cdm init` are built; the OMOCL engine
that maps a composition to its rows is #90, and `ferrobridge etl run` names
that issue until it lands.

<!-- toc -->

## A batch job, not a listener

The ETL runs as a job over AQL. It queries the CDR with `POST /query/aql`,
converts each EHR's compositions through the loaded OMOCL files, and writes
typed CDM rows. It pulls; the CDR does not push.

That follows from the specification surface rather than from taste: change
notification is not part of openEHR ITS-REST 1.1.0, so there is nothing
conformant to subscribe to. If FerroBRIDGE later consumes a particular CDR's
change events, that is a FerroBRIDGE extension, labelled as one, and the batch
path stays the conformant baseline.

## The tables in play

PERSON, OBSERVATION_PERIOD, VISIT_OCCURRENCE, VISIT_DETAIL,
CONDITION_OCCURRENCE, DRUG_EXPOSURE, PROCEDURE_OCCURRENCE, DEVICE_EXPOSURE,
MEASUREMENT, OBSERVATION, DEATH, SPECIMEN, FACT_RELATIONSHIP, NOTE, and
NOTE_NLP, beside the vocabulary tables.

Some of those come from mappings and some are derived. PERSON rows come per
EHR. VISIT_OCCURRENCE comes from a configured AQL query, checked when the
configuration loads, grouped by `ehr_id` and source. OBSERVATION_PERIOD is
derived from the first and last clinical event per person, and the ERA tables
run the SQL the CDM publishes. The CDM leaves each of these to the ETL's
discretion, so each is labelled as FerroBRIDGE's own.

An OMOCL record names its target table in `type`. The CDM says the domain of
the resolved standard concept decides the table, so FerroBRIDGE validates the
two against each other: a literal concept whose domain disagrees with `type`
refuses the mapping at load, and a resolved concept whose domain disagrees
refuses the record. A row is never moved to a table the mapping did not name.

## The row types are generated

The CDM v5.4 row types are generated from the OHDSI `CommonDataModel` field
definitions, which publish the model in machine-readable form. The PostgreSQL
DDL is not regenerated: OHDSI renders it through a dialect layer that sits
outside those definitions, so the rendered files are vendored verbatim and
applied by `ferrobridge cdm init`, and a test asserts the generated columns
equal the DDL's. Nothing hand-transcribes a column list.

## Concept resolution is SQL, not terminology

A source code becomes a `concept_id` through the locally loaded OHDSI
vocabulary: the CONCEPT table, the `standard_concept` flag, the domain, and the
CONCEPT_RELATIONSHIP "Maps to" traversal. This is SQL over the vocabulary
tables in the same database as the CDM rows. It never goes to the FHIR
terminology server, which is a different component serving the FHIR side.

The lookup requires a valid concept (`invalid_reason` empty, the validity
dates containing the record date) and a deterministic order, and an ambiguous
match is an error rather than the first row. An unmapped code lands as
`concept_id = 0` with the source value kept in its source column and is counted
in the run report. The CDM defines `0` as "no matching concept", and keeping
the source value is what makes the gap auditable later.

Each composition's rows are written together or not at all, through binary
`COPY`, and a re-run of the same input replaces its rows rather than adding
duplicates.

## Keys, re-runs and watermarks

Every CDM v5.4 primary key is a 32-bit `integer`, so the bridge assigns ids
from one PostgreSQL sequence per table, kept in a schema of its own beside the
CDM (`[cdm] bridge_schema`, `ferrobridge` by default). A side table there maps
each row's natural key (the EHR, the versioned composition, the archetype root
and the occurrence) to its id, so a later version of a composition keeps the
ids of the rows it still has and deletes the ones it no longer has. One
`ehr_id` is one `PERSON`; the bridge does not reconcile a person across EHRs.
An exhausted sequence refuses the composition with a typed error.

Each committed composition leaves a watermark naming the version written.
`ferrobridge etl run --resume` skips every composition whose watermark names
the version the query answers and runs the rest, so a run that stopped part
way through continues exactly. A composition the mapper or the database
refuses is rolled back, counted in the run report with the reason, and the
run goes on.

The run report counts, per mapping and in total, the rows written per table,
the `concept_id = 0` assignments, the `FACT_RELATIONSHIP` rows written with
`relationship_concept_id = 0`, the source fields no mapping wrote, and every
refused record with its table, column and openEHR element. `etl run` prints it
as text and as JSON.

## The derived tables

`OBSERVATION_PERIOD` spans the first to the last clinical event of each
person, one period per person. The CDM's EHR guidance suggests more (an end
60 days after the last event, gaps from a persistence window); FerroBRIDGE
applies none of it, and says so.

`CONDITION_ERA` and `DRUG_ERA` run the two scripts on the CDM's SQL scripts
page. The page publishes them in OHDSI SQL for SqlRender, which PostgreSQL does
not run, so the bridge carries their PostgreSQL form, changed only in dialect,
and a test fails when the published scripts change. The drug era script reads
ingredients of the `RxNorm` vocabulary only, as published.

`VISIT_OCCURRENCE` comes from the `[etl.visits]` query, grouped by EHR and
source, from the earliest start to the latest end. A time with an offset keeps
its wall-clock time, because the CDM `datetime` has no zone. The visit concept
and the visit type concept are configuration with no default.

## Validating OMOCL files

OMOCL publishes no JSON schema, so FerroBRIDGE authors one from the grammar
tables and the published mapping library, validates every file against it
before running anything, and offers that schema upstream. The grammar also
leaves the reading of `alternatives` open; FerroBRIDGE tries them in order and
the first present value wins, which is the only reading the published library
is consistent with. A `CustomMapping` names a converter compiled into the
binary, and an unknown name refuses the file. A mapping that fails validation
is refused at load time.

## What you need in place

A PostgreSQL CDM v5.4 database built from the OHDSI DDLs, an Athena vocabulary
export loaded into it, a CDR to read from, and the OMOCL files for the
archetypes you care about. The [deployment shape](../operate/deployment-shape.md)
page lists the neighbours in full.
