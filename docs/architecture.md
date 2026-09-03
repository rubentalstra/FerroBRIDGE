<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Architecture

FerroBRIDGE is a standalone bridge between openEHR and two interoperability
targets: HL7 FHIR, driven by the FHIRconnect specification, and the OMOP Common
Data Model, driven by the OMOCL specification. It runs beside an openEHR CDR
that it reaches only over the openEHR ITS-REST API. This document records the
design ground established by the 2026-09-03 research pass over the primary
sources (recorded on issue #1) and the decisions taken on it. Where a
specification is silent, this document says so and names the decision as
FerroBRIDGE's own.

## 1. The two specifications are two languages

FHIRconnect and OMOCL are written by the same author and share one header
(`grammar`, `type`, `metadata.name`/`version`, `spec`, `spec.openEhrConfig`
pinning the archetype and revision; the FHIRconnect header page states the
header "is standardized for both FHIRconnect and OMOCL"). Below the header they
share RM paths with `../` navigation, archetype-keyed model files, an
include/slot mechanism (`slotArchetype` in FHIRconnect, `Include` in OMOCL), an
external-code escape hatch (`mappingCode`, `CustomMapping`) and a
code-translation concept (`conceptmap`, `conceptMap`). Nothing else is common:

- **FHIRconnect** (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/index.html>)
  is bidirectional. A mapping is a tree of entries pairing a FHIR path with an
  openEHR path (`with`), typed from the instances, with `followedBy` nesting
  that concatenates paths onto the parent, AND-combined conditions applied to
  the input side only, `manual` literals, `reference` resources, `hierarchy`
  realignment and `unidirectional` markers. Three file types: `model`
  (archetype to unprofiled resource, the shared layer), `extension`
  (profile/template adaptation, executed after the model), `context` (the
  entry point: profile, template, archetypes, extensions, `start`).
- **OMOCL** (<https://github.com/SevKohler/OMOCL>) is unidirectional openEHR to
  OMOP by construction. A mapping is a flat list of records, each keyed by a
  CDM target (`Measurement`, `ConditionOccurrence`, `Person`, ...), whose keys
  are CDM column names taking ordered `alternatives` of an RM `path` or a
  literal `code` (an OMOP `concept_id`; `0` is the CDM's "no matching
  concept"), plus `optional`, inline `conceptMap` tables, `Include` and
  `CustomMapping`. The published files use YAML anchors and aliases.

**Decision:** one shared foundation, two interpreters, two sinks. A single
intermediate representation over both grammars would be the union of two
dissimilar languages and would be wrong the first time either specification
moved.

## 2. Pinned versions

| Component | Pin | Ground |
|---|---|---|
| FHIRconnect | v1.0.0 | the only released version; both published JSON schemas (`model-mapping.schema.json`, `contextual-mapping.schema.json`, draft-07) are the validation oracle |
| FHIR | R4 (4.0.1) | the only value the FHIRconnect schemas admit for `spec.version`; the mapping library targets R4; the prose says other releases "should" work but nothing is tested |
| OMOCL | v1.0.0 (`grammar: OMOCL/v1.0.0`) | the only released grammar; no JSON schema is published (railroad diagram plus syntax tables; lexer and parser marked work in progress) |
| OMOP CDM | v5.4 | the only version OMOCL files declare (`spec.system: OMOP`, `spec.version: 5.4`) and the only one Eos supports; DDLs and the machine-readable table definitions come from <https://github.com/OHDSI/CommonDataModel> |
| openEHR ITS-REST | 1.1.0 | the released REST API the CDR speaks (FerroEHR's pin) |
| FHIR terminology operations | R4/R4B tolerant | `CodeSystem/$lookup`, `ConceptMap/$translate`, `ValueSet/$validate-code`; FerroTERM answers R4B |

Later FHIR releases are feature-gated in the model crate and claimed only once a
mapping corpus proves them.

## 3. The openEHR side

FHIRconnect and OMOCL paths are RM/archetype paths (`$archetype/data[at0001]/items[at0077]`),
never FLAT paths. A path alone is not executable: leaf RM types and
template-specific identifiers come from the **Web Template** built from the
template's OPT. The ordered dependency is OPT, then Web Template, then path
resolution, then composition. openFHIR (the FHIRconnect reference engine)
resolves this way and converts to FLAT for the wire; FerroBRIDGE resolves the
same way using the `openehr-*` crates (OPT 1.4 ingestion, the Web Template
builder and the FLAT/STRUCTURED codecs already exist there) and commits
canonical JSON or FLAT as the CDR advertises.

The ITS-REST surface the bridge consumes: `POST /ehr` and
`GET /ehr?subject_id=&subject_namespace=`; `POST|PUT|GET /ehr/{ehr_id}/composition[/{uid}]`;
`POST /ehr/{ehr_id}/contribution` for atomic multi-object commits;
`POST /query/aql`; `GET /definition/template/adl1.4/{template_id}` for the OPT
and, with `Accept: application/openehr.wt+json`, the Web Template.

Composition-level fields have no FHIR counterpart. FHIRconnect requires the
`start` mapping to slot a reusable `COMPOSITION.<archetype>.<Resource>` mapping
in and defaults composer, `context/start_time`, `setting`, `language`,
`territory` and `category`; the bridge follows those defaults and records every
defaulted value in the composition's `FEEDER_AUDIT`.

## 4. The FHIR side

- **Facade, not store.** FerroBRIDGE exposes a FHIR R4 REST facade (create,
  update, read, transaction and batch Bundles with `If-None-Exist`/`If-Match`
  conditional forms) and maps each request onto CDR operations. It stores no
  clinical data of its own. This is the EHRbase FHIR Bridge shape; openFHIR
  instead leaves both the facade and the CDR to the integrator. No standard
  prefers either; the facade is chosen so a FHIR client sees one server.
- **Bundles** are split by context profile URL per the specification's
  engine chapter; a resource two mappings both need is a LINKED mapping;
  unresolved references are fetched from the sending site with cycle
  protection.
- **Paths are written, not only read.** The engine is a bidirectional path
  model over FHIR JSON. FHIRPath evaluation is used only for read-side
  expressions the mappings contain (`ofType()`, `extension(url)`,
  `resolve()`); FHIRconnect's `^` parent operator and `$fhirRoot` are not
  FHIRPath and are handled by the path model.
- **Search is unspecified** by FHIRconnect ("does not focus specifically on
  AQL and FHIRsearch"). FerroBRIDGE's search is its own design: an AQL
  projection per resource type declared beside the context mapping, executed
  over `POST /query/aql`, with the bridge stating in its CapabilityStatement
  exactly which search parameters each resource supports and refusing the
  rest with `OperationOutcome`. Labelled as FerroBRIDGE's extension wherever it
  appears.

## 5. The OMOP side

OMOP is a relational analytics schema, not an API: the deliverable is rows in
a populated CDM v5.4 database (PERSON, OBSERVATION_PERIOD, VISIT_OCCURRENCE,
VISIT_DETAIL, CONDITION_OCCURRENCE, DRUG_EXPOSURE, PROCEDURE_OCCURRENCE,
DEVICE_EXPOSURE, MEASUREMENT, OBSERVATION, DEATH, SPECIMEN, FACT_RELATIONSHIP,
NOTE, NOTE_NLP, beside the vocabulary tables). Eos, the OMOCL reference engine,
is a pull-based batch ETL: PERSON rows per EHR, each EHR's compositions
converted through the loaded OMOCL files, VISIT_OCCURRENCE from a configured
AQL query grouped by `ehr_id` and source, OBSERVATION_PERIOD and the ERA
tables derived rather than mapped.

**Decisions:**

- The OMOP engine is a batch ETL job runner over AQL (`POST /query/aql`),
  writing typed CDM rows into a PostgreSQL CDM database built from the OHDSI
  DDLs, with the derived tables generated the way Eos generates them.
- An incremental mode has no specification behind it (change notification is
  not part of ITS-REST 1.1.0). If FerroBRIDGE consumes a CDR's change events,
  that is FerroBRIDGE's own extension, labelled as such, and the batch path
  stays the conformant baseline.
- **OMOP concept resolution is not a FHIR terminology operation.** A source
  code becomes a `concept_id` through the locally loaded OHDSI vocabulary
  (Athena export): CONCEPT, `standard_concept`, domain and the
  CONCEPT_RELATIONSHIP "Maps to" traversal. This lives in the OMOP crate as SQL
  over the vocabulary tables; the FHIR terminology client is a different
  component. Unmapped codes land as `concept_id = 0` with the source value
  kept, never dropped.
- OMOCL has no JSON schema. FerroBRIDGE authors one from the grammar tables and
  the 196-file library, validates every mapping against it, and offers it
  upstream.

## 6. Terminology

The FHIR side needs three operations on a FHIR terminology server: `$lookup`
(the specification recommends resolving a code's display through a terminology
server because `DV_CODED_TEXT.value` is mandatory while FHIR `display` is not),
`$translate` for `conceptmap` references, and `$validate-code`. FerroTERM is the
reference server; the client is version-tolerant across R4 and R4B and is
configured, never assumed. The OMOP side uses the local vocabulary tables
(section 5).

## 7. Workspace layout

Apache-2.0 throughout, matching every upstream artefact. The generated-versus-
hand-written split FerroEHR and FerroTERM use applies where there is a
machine-readable source:

| Crate | Role | Kind |
|---|---|---|
| `openehr-mapping-core` | the shared header model, the YAML loader (anchors and aliases), the archetype-keyed mapping registry with include/slot resolution, the RM-path model | hand-written |
| `openehr-path` | RM path parsing; OPT to Web Template to leaf-type resolution; RM-path to FLAT conversion, over the `openehr-*` crates | hand-written |
| `fhirconnect-model` | the FHIRconnect AST; validation against both published schemas plus the constraints they cannot express (`targetRoot` alignment, `appendTo`/`slotArchetype` resolution, `criteria` presence) | hand-written, schema-validated |
| `fhirconnect-resolve` | context resolution: model plus extensions flattened into one immutable program per (profile, template); the extension ordering the specification leaves open is pinned here | hand-written |
| `fhirconnect-engine` | the bidirectional interpreter: top-down traversal, `followedBy` concatenation, input-side conditions, recurrence, `manual` merge, deterministic id derivation | hand-written |
| `omocl-model` | the OMOCL AST and the authored JSON schema | hand-written, schema-validated |
| `omocl-engine` | the one-directional interpreter emitting typed CDM rows | hand-written |
| `omop-cdm` | CDM v5.4 row types and DDL generated from the OHDSI `CommonDataModel` CSV definitions; the Athena vocabulary loader and concept resolver; the derived-table generators | generated + hand-written |
| `ferrobridge-openehr` | the ITS-REST client (section 3) | hand-written |
| `ferrobridge-term` | the FHIR terminology client (section 6) | hand-written |
| `ferrobridge-server` | the FHIR R4 facade, the OMOP ETL job runner and its API, the id-map store | hand-written |

**The FHIR model is a dependency, not a generator.** A second FHIR code
generator buys the bridge nothing; a maintained crate with generated R4 types
is taken instead (candidates recorded on issue #1), and the choice is
re-adjudicated if it stops tracking FHIR releases.

## 8. Identity, failure and the specification's recommendations

The FHIRconnect engine chapter is explicitly "recommendations". FerroBRIDGE
pins each as its own decision:

- **Identity.** A persistent id-map store: patient identifier to `ehr_id`,
  external to internal resource id, and the exported resource id derived
  deterministically as a hash of composition UID plus entry path, as the
  specification recommends. The acknowledged failure mode (a re-sent Bundle
  omitting one resource reads as a different mapping) is documented, not hidden.
- **Failure policy.** All-or-nothing per request: a Bundle that cannot be
  mapped in full is refused with an `OperationOutcome` naming every failing
  entry, and nothing is committed.
- **Type coercion** is strict: a value that does not parse as the resolved RM
  leaf type is a refusal, never a best-effort conversion.
- **PROGRAMMED mappings** (`mappingCode`, `CustomMapping`) are named Rust
  functions registered at build time; there is no runtime plugin loading.
- **Version mismatch** between mapping version, grammar version, archetype
  revision, template `sem_ver` and profile version is a refusal at load time.

## 9. Verification

- FHIRconnect and OMOCL mapping libraries are vendored verbatim by committed
  `scripts/vendor/*.sh` scripts with provenance, and exercised in full: every
  file loads and validates, or carries an adjudicated skip with its reason.
- The first milestone is one verbatim round trip per target: the published
  `EVALUATION.problem_diagnosis.v1` FHIRconnect mapping with an R4 `Condition`
  committed to a CDR over ITS-REST, read back and mapped to FHIR again, equal
  modulo the fields the specification defaults; and the published laboratory
  OMOCL files emitting MEASUREMENT rows into a CDM 5.4 database with a real
  Athena vocabulary loaded, with resolved `concept_id`s asserted and unmapped
  codes landing as `0`.
- FerroEHR is the reference CDR in the composed test stack, reached over
  ITS-REST only; FerroTERM is the reference terminology server.

## 10. What is deliberately outside

Demographics (FHIRconnect sends resources to an unspecified external
endpoint), FHIR Subscriptions (never mentioned), a materialised FHIR store, and
OMOP CDM versions other than 5.4. Each is a tracker issue, not silence.
