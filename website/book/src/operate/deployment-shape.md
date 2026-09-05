<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# What FerroBRIDGE runs beside

FerroBRIDGE is not released yet, so there is nothing to install today. What is
decided is the shape of the deployment: which systems the bridge talks to, what
it stores, and what you have to supply. Read this page to plan; the commands
arrive with the first release.

<!-- toc -->

## One process, four neighbours

FerroBRIDGE is its own process and its own image. It is not a crate inside a
CDR, not a plugin, and not a library a CDR links. It reaches every neighbour
over a network protocol, which is what lets it work against a CDR it did not
build.

| Neighbour | Protocol | Needed by |
|---|---|---|
| An openEHR CDR | openEHR ITS-REST 1.1.0 | both sides |
| A FHIR terminology server | FHIR terminology operations, R4 or R4B | the FHIR side |
| An OMOP CDM v5.4 database | SQL, PostgreSQL | the OMOP side |
| The OHDSI vocabulary, loaded into that database | SQL | the OMOP side |

You run each of these, or you point FerroBRIDGE at ones you already run. None
of them is a compile-time dependency, and the bridge treats any conformant
implementation the same way.

## What FerroBRIDGE stores

Clinical data lives in the CDR and, on the OMOP side, in the CDM database.
FerroBRIDGE stores two things of its own: the id-map, a persistent store
holding the patient identifier to `ehr_id` relation, the external to internal
resource id relation and the resource id to composition relation; and, on the
OMOP side, a side table in its own schema beside the CDM that maps each source
record to its rows, which is what makes a re-run replace rather than
duplicate. Everything else it holds is configuration: the loaded
mapping files, the terminology server address, and the CDR address.

## What it asks of the CDR

The FHIR side uses the composition and EHR endpoints:
`POST /ehr` and `GET /ehr?subject_id=&subject_namespace=`,
`POST`, `PUT`, and `GET` on `/ehr/{ehr_id}/composition[/{uid}]`, and
`POST /ehr/{ehr_id}/contribution` for an atomic multi-object commit. Both sides
use `POST /query/aql`. Templates come from
`GET /definition/template/adl1.4/{template_id}` as canonical XML; FerroBRIDGE
builds the Web Template from that itself, so the CDR need not serve one.
Compositions cross the wire as canonical JSON.

A CDR that does not serve the operational template cannot drive the bridge,
because a mapping path alone does not carry the leaf RM type.

## What it asks of the terminology server

Three operations: `CodeSystem/$lookup`, `ConceptMap/$translate`, and
`ValueSet/$validate-code`. `$lookup` matters most, because openEHR's
`DV_CODED_TEXT.value` is mandatory while FHIR's `display` is not, so a display
often has to be resolved rather than copied. The client tolerates a server
answering R4 or R4B.

## What the OMOP side asks of the database

A CDM v5.4 schema built from the OHDSI DDLs, and an Athena vocabulary export
loaded into the vocabulary tables. Concept resolution is SQL over CONCEPT,
`standard_concept`, the domain, and the CONCEPT_RELATIONSHIP "Maps to"
traversal. Without a loaded vocabulary every source code resolves to
`concept_id = 0`, which is legal in the CDM and useless for analysis.

## Process model

No specification governs the process model, the storage mechanics, or the
deployment topology: that is FerroBRIDGE's own design. The decisions on record
are a single binary, a container image, and a batch ETL that runs as a job
rather than as a listener. Change notification is not part of ITS-REST 1.1.0,
so an incremental OMOP mode would be a FerroBRIDGE extension, labelled as one,
with the batch path staying the conformant baseline.
