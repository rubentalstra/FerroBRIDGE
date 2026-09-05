<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Architecture

FerroBRIDGE is a standalone bridge between openEHR and two interoperability
targets: HL7 FHIR, driven by the FHIRconnect specification, and the OMOP Common
Data Model, driven by the OMOCL specification. It runs beside an openEHR CDR
that it reaches only over the openEHR ITS-REST API. This document is the design
of record. It is the output of two research passes over the primary sources,
on 2026-09-03 and 2026-09-05, whose full reports are recorded on issue #1. Every
decision below names its ground. Where a specification is silent, the decision
is labelled as FerroBRIDGE's own. Where a specification contradicts itself or
another specification, the contradiction is named and carried as an
`upstream-report` issue.

The second pass changed six things the first pass had wrong or had not seen:
the FHIR model crate the facade was going to stand on carries no clinical
resources, so the crate and its generator move into this repository; the
FHIRconnect JSON schemas reject three of the specification's own mapping types and 24 of the 106 published library files; the FHIRconnect
`../` operator is not openEHR path syntax; the OMOCL column vocabulary is the
language's own, not the CDM's; every OMOP CDM v5.4 primary key is a 32-bit
integer; and the openEHR crates already carry a composition builder and the
ITS-REST data types, so the openEHR layer is thinner than planned in one place
and thicker in another. Sections 3, 4, 5, 7 and 8 carry the corrections.

## 1. The two specifications are two languages

FHIRconnect and OMOCL are written by the same author and share one header
(`grammar`, `type`, `metadata.name` and `metadata.version`, `spec` with
`spec.openEhrConfig.archetype`; the FHIRconnect header page states the header
"is standardized for both FHIRconnect and OMOCL"). Below the header they share
RM paths, archetype-keyed model files, an include mechanism (`slotArchetype`
in FHIRconnect, `Include` in OMOCL), an external-code escape hatch
(`mappingCode`, `CustomMapping`) and a code-translation concept (`conceptmap`,
`conceptMap`). Both also use a `../` parent step inside a path. That step is
the mapping languages' own operator: the openEHR path grammar (BASE Release
1.2.0 §Paths and Locators, <https://specifications.openehr.org/releases/BASE/Release-1.2.0/architecture_overview.html#_paths_and_locators>)
defines absolute paths, relative paths, predicates and the XPath `//` pattern,
and no parent step. FHIRconnect's claim that "`../` is used in openEHR" is
unsupported and is reported upstream. The engine resolves `../` against the
mapping's anchor before any path reaches a CDR.

Nothing else is common:

- **FHIRconnect** (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/index.html>)
  is bidirectional. A mapping is a tree of entries pairing a FHIR path with an
  openEHR path (`with`), typed from the instances, with `followedBy` nesting
  that concatenates paths onto the parent, AND-combined conditions applied to
  the input side only, `manual` literals, `reference` resources, `hierarchy`
  realignment and `unidirectional` markers. Three file types: `model`
  (archetype to unprofiled resource, the shared layer), `extension`
  (profile and template adaptation, executed after the model), `context` (the
  entry point: profile, template, archetypes, extensions, `start`).
- **OMOCL** (<https://github.com/SevKohler/OMOCL>) is unidirectional openEHR to
  OMOP by construction. A mapping is a flat list of records, each keyed by a
  `type` naming a CDM target (`Measurement`, `ConditionOccurrence`, `Person`,
  seven more) or one of two structural types (`Include`, `CustomMapping`).
  A record's keys are OMOCL's own column vocabulary (`concept_id`, `value`,
  `unit`, `measurement_date`, `gender_concept`), each taking ordered
  `alternatives` of an RM `path`, a literal `code` (an OMOP `concept_id`; `0`
  is the CDM's "no matching concept"), an inline `conceptMap`, or a
  `multiplication`, plus `optional`. The projection from an OMOCL key to CDM
  columns (`value` to `value_as_number`, `unit` to `unit_concept_id`,
  `concept_id` to `measurement_concept_id` and `measurement_source_concept_id`)
  is stated nowhere in OMOCL; four of the ten targets have a published syntax
  table, and the library is the only evidence for the rest. The published
  files use YAML anchors and aliases (57 of 202 files define one).

**Decision:** one shared foundation, two interpreters, two sinks. A single
intermediate representation over both grammars would be the union of two
dissimilar languages and would be wrong the first time either specification
moved. The second pass confirmed the split from the other side: the OMOCL
library's semantics (ordered first-match alternatives, a five-attribute
projection out of one `DV_QUANTITY` node, dates taken from an enclosing
context) have no counterpart in FHIRconnect, and FHIRconnect's recurrence
model has none in OMOCL.

## 2. Pinned versions

The pins live in `docs/VERSIONS.md`; this table records the ground for each.
A corpus is pinned by commit, never by a moving tag or a `latest` URL.

| Component | Pin | Ground |
|---|---|---|
| FHIRconnect | v1.0.0 (specification source `SevKohler/FHIRconnect-spec` at `195b07fdb4c78da0432fdd1e9dbd127b81be6165` (2026-07-22); `model-mapping.schema.json` sha256 `6a925151c029e10ef11ccfc2eaafbbf441eea97ffec8ea9e0cd0b8493871d852`; `contextual-mapping.schema.json` sha256 `a96d600dfa7faacb1b2d8919a20554bed5699a636efd7f903d601b71cae15832`) | the only released version; both published draft-07 schemas are the machine-readable half of the authority and are vendored verbatim; they are defective in seven places (section 4.2), so they are exercised as evidence, never as the sole validator |
| FHIRconnect mapping library | `SevKohler/FHIRconnect-mapping-lib` at `6bd4c19a2f96821c04fbeed3c6f6c190fd85825b` (2026-06-05), 106 mapping files | the conformance corpus, never an oracle: 24 files fail the published schema, 5 cross-references dangle, 6 files share one `metadata.name`; the library is seven weeks behind the specification and covers none of its operational-mapping chapter |
| FHIR | R4 (4.0.1), package `hl7.fhir.r4.core` 4.0.1 (CC0) | the only value the FHIRconnect schemas admit for `spec.version`; the mapping library targets R4; the prose says other releases "should" work and nothing is tested |
| OMOCL | v1.0.0 (grammar `OMOCL/v1.0.0`; corpus `SevKohler/OMOCL` at `dd42574fdb074c02cbe077a0c49b1bb5bae28f35`, 2026-04-26, 202 files, Apache-2.0) | the only released grammar. The git tag `v1.0.0` carries pre-grammar files headed `engine: EOS/v0.0.62`; the grammar string first appears at tag `v1.0.1`, so the corpus is pinned by commit. No JSON schema is published; the grammar is two railroad images and four syntax tables |
| OMOP CDM | v5.4 (`OHDSI/CommonDataModel` tag `v5.4.3`, 2026-08-04; licence Apache License 2.0 per `DESCRIPTION`, the repository has no `LICENSE` file) | the only version OMOCL files declare (`spec.system: OMOP`, `spec.version: 5.4`) and the only one the reference engine supports; the CSV table and field definitions and the rendered PostgreSQL DDL are the machine-readable input. v5.5.0 shipped 2026-08-25 and is tracked, never assumed |
| openEHR ITS-REST | 1.1.0 (OpenAPI at `openEHR/specifications-ITS-REST` tag `Release-1.1.0`, CC-BY-ND-3.0; modules EHR, Query and Definition, `STABLE`) | the released REST API a conformant CDR speaks; Admin and Demographic are `x-status: DEVELOPMENT` in the same release and the bridge does not depend on them; every tagged OAS file says `info.version: latest`, so provenance records tag and blob |
| `openehr-base` | 0.0.61 (minor line 0.0; Apache-2.0) | the RM foundation types, including partial ISO 8601 dates (section 3) |
| `openehr-rm` | 0.0.61 (Apache-2.0) | the RM 1.1.0 model, its canonical JSON codec and the BASE path parser |
| `openehr-its` | 0.0.61 (BUSL-1.1 AND Apache-2.0) | the OPT 1.4 codec, the Web Template builder, the FLAT and canonical JSON codecs, the composition builder, the ITS-REST 1.1.0 data types |
| `openehr-query` | 0.0.61 (BUSL-1.1) | the AQL 1.1.0 parser and canonical printer. `openehr-term` (Apache-2.0 AND CC-BY-SA-3.0), `openehr-am` and `openehr-lang` arrive transitively |
| `fhir-types` | 0.1.43 is the last release from the sibling terminology server and the floor of this repository's crate line | the FHIR model, generated here from the vendored HL7 packages from the first bridge release on (section 4.1); the sibling then consumes it from crates.io |
| FHIR terminology operations | R4, R4B tolerant | `CodeSystem/$lookup`, `ConceptMap/$translate`, `ValueSet/$validate-code`; a server may answer R4 or R4B |

## 3. The openEHR side

FHIRconnect and OMOCL paths are RM and archetype paths
(`$archetype/data[at0001]/items[at0077]`), never FLAT paths. A path alone is
not executable: leaf RM types, occurrence limits and template node identifiers
come from the **Web Template** built from the template's OPT. The ordered
dependency is OPT, then Web Template, then path resolution, then composition.

**What the openEHR crates provide, verified in the 0.0.61 sources.**
`openehr-its` parses OPT 1.4 (`opt14`), builds the Web Template in the
Better and EHRbase shape (`tree`, `id`, `rmType`, `aqlPath`, `inputs`),
converts between FLAT, STRUCTURED and canonical JSON, and builds a canonical
composition from a set of path and value pairs (`flat::build`). `openehr-rm`
carries the RM 1.1.0 model with its canonical JSON codec and the BASE path
parser with PATHABLE navigation (`v1_2::paths`). `openehr-query` parses and
prints AQL 1.1.0. `openehr-its` also carries the ITS-REST 1.1.0 request and
response types generated from the vendored OpenAPI.

**What the bridge writes itself.** Three things the crates do not export: an
`aqlPath` index over the Web Template with leaf RM type resolution, the
parent-to-child relative path derivation (the crate keeps its own copy
private), and the HTTP client (the crates generate data types and server
traits, no client). `openehr-adl` is not needed: the bridge reads OPT 1.4 from
the CDR and never sees ADL source.

**The Web Template is built locally, never fetched.** The crate's `WebTemplate`
type serialises and does not deserialise, so the ITS-REST
`Accept: application/openehr.wt+json` form cannot be consumed. The bridge
fetches the OPT (`GET /definition/template/adl1.4/{template_id}`, canonical
XML) and builds the Web Template with the crate. This is also the safer path:
the FLAT node-id uniqueness rule in the Simplified Formats specification does
not fix sibling order, so two conformant servers can name the same node
differently, and a bridge that regenerated node ids against a foreign Web
Template would mis-key values.

**The wire is canonical JSON.** Canonical JSON is the mandatory composition
representation in ITS-REST 1.1.0; FLAT and STRUCTURED are optional. The
mapping engines resolve paths against the canonical RM tree (the approach the
OMOCL reference engine takes, in 56 lines against 534 for the FLAT route in the
FHIRconnect reference engine), build the composition with the crate's builder,
and commit and read canonical JSON. FLAT never crosses the wire. Two index
bases meet here and must never be confused: RM positional predicates are
1-based (BASE §Paths and Locators), FLAT `:n` indices are 0-based.

**The ITS-REST surface the bridge consumes:** `POST /ehr`,
`GET /ehr?subject_id=&subject_namespace=`, `GET /ehr/{ehr_id}`;
`POST /ehr/{ehr_id}/composition`, `PUT /ehr/{ehr_id}/composition/{versioned_object_uid}`
with `If-Match`, `GET /ehr/{ehr_id}/composition/{uid_based_id}`;
`POST /ehr/{ehr_id}/contribution` for atomic multi-object commits;
`POST /query/aql`; `GET /definition/template/adl1.4/{template_id}`
(<https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/ehr.html>,
<https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/query.html>,
<https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/definition.html>).
Four facts of that surface shape the client: `Prefer` is always sent
explicitly, because the specification warns its default may change;
`If-Match` carries the bare quoted `version_uid` and the `W/` the `ETag`
carries is stripped; `DELETE` reports a concurrency failure as `409` where
`PUT` reports `412`; and `GET …?version_at_time=` answers `204` for a deleted
composition, which the client surfaces as a typed "deleted" outcome, never as
an absent value. The commit metadata headers (`openEHR-VERSION`,
`openEHR-AUDIT_DETAILS`, `openEHR-TEMPLATE_ID`) exist only in the prose and
are absent from the OpenAPI; the client sends them from the prose and the gap
is reported upstream.

Composition-level fields have no FHIR counterpart. FHIRconnect requires the
`start` mapping to slot a reusable `COMPOSITION.<archetype>.<Resource>` mapping
in and recommends defaults for composer, `context/start_time`, `setting`,
`language`, `territory` and `category`
(<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/engine/defaults-for-fields.html>).
The defaults are deployment configuration, never engine constants (the
reference engine hard-codes `territory=DE`), and every defaulted value is
recorded in the composition's `FEEDER_AUDIT`, which the RM defines for exactly
this case (RM Common §FEEDER_AUDIT).

## 4. The FHIR side

### 4.1 The FHIR model

The first pass recorded that the published `fhir-types` crate "already carries
per-version FHIR types" for the facade. It does not. `fhir-types` 0.1.43
carries the terminology root set (`CodeSystem`, `ValueSet`, `ConceptMap`,
`TerminologyCapabilities`, `CapabilityStatement`, `Bundle`, `Parameters`,
`OperationOutcome`) and the closure of datatypes they reference, with a strict
codec (unknown properties refused, lexical primitives kept, choice types
handled) and an element table the XML codec reads. It has no `Condition`,
`Observation`, `Patient` or any other clinical resource.

**Decision (owner, 2026-09-05): `fhir-types` and its generator move into this
repository, and the bridge generates the shared FHIR model.** The crate and
`fhir-codegen` come from the sibling terminology server together with their
vendored HL7 packages (R4, R4B, R5, R6, THO), their drift gate and their tests;
the sibling then consumes `fhir-types` from crates.io behind a terminology-only
feature set, so its build does not grow. The generator's declared root set
widens to every resource in each package, gated by a `resources` feature, with
per-version features so a consumer pays for one version; its element table
(type name, element, kind, choice alternatives, `min` and `max`) becomes public
API, because that table is what a path-driven engine needs most; and the codec
gains a `serde_json::Value` entry point. The ground for the move: the bridge is
the crate's widest consumer and will drive its evolution from here on (146
resources, the element table, the `Value` codec), the sibling's needs are eight
resources and a stable set of operation contracts, and the bridge takes no
other crate from the sibling once `fhir-terminology` is struck, so the
dependency is a straight line. The recorded costs: about 380 MB of vendored
packages including versions the bridge does not use, terminology operation
descriptors emitted for the sibling's sake, and a `fhir-types` fix the sibling
needs riding this repository's crate lane (a same-day dispatch publish, never a
product release). The crate line continues from 0.1.43.

Rejected: widening the crate in place in the sibling (its tracker records the
request, closed as not planned: the widest consumer would then depend on the
narrowest one's release lane); a separate repository for the crate (refused by the owner); the
`fhir` crate (146 R4 resources, one author, 407 downloads in 90 days, a
five-way licence disjunction, and the owner's own read of its code quality);
`fhir-model` 0.13.0 (R4B and R5 oriented, no element table); `helios-fhir`
0.2.1 (4 MB, no declared MSRV, its FHIRPath sibling is a binary's worth of
dependencies); every FHIRPath evaluator as a model source (evaluation only).

`fhir-terminology` is struck from the design. It is a terminology server over
a code-system provider seam and its manifest pulls in the SNOMED, ICD-11 and
RxNorm loaders and the concept store. The client contracts the bridge speaks
(`CodeSystemLookupRequest`, `ConceptMapTranslate*`, `ValueSetValidateCode*`,
with `from_parameters` and `to_parameters`) live in `fhir-types::r4::operations`.

### 4.2 The mapping files: how a FHIRconnect file is validated

The published schemas are defective in ways that matter. The model schema sets
`additionalProperties: false` on a mapping and omits `mappingCode`, `link`,
`participationsFunction` and mapping-level `conceptmap`, so three of the eight
concept-type mappings the prose defines (PROGRAMMED, LINKED, PARTICIPATION)
cannot appear in a schema-valid file, and 24 of the 106 library files fail
their own schema. `manual` is typed as an array with object `properties` and no
`items`, so it validates nothing. `hierarchy.split.openehr` sits outside
`properties` and is unvalidated. The `type` enum sits at mapping level where no
file writes it; every file writes `type` inside `with`, where the schema leaves
it a free string. `operator` and `unidirectional` have no enum. Conditions are
documented as an array ("Notice that the condition is an array") and typed as
an object. Each is an `upstream-report` issue.

**Decision:** the AST is hand-written Rust, and validation is three layers:

1. **The published schemas, vendored verbatim and exercised.** A test validates
   every corpus file against them and pins the exact set the schemas reject.
   That test is the durable record of the contradiction; it passes when the
   published schema still rejects what the prose allows, and it fails the day
   upstream fixes the schema, which is the signal to re-adjudicate.
2. **FerroBRIDGE's own strict schemas** for both file types, authored from the
   prose plus the published schema, closed (`additionalProperties: false` at
   every level, enums for `operator`, `unidirectional`, `extension`, the
   data-type `type` inside `with`), offered upstream. Every corpus file either
   validates or carries an adjudicated skip naming the defect.
3. **Semantic validation the schema cannot express:** `targetRoot` resolves to
   the same node as `with`; `criteria` is absent for `empty` and `not empty`
   and present otherwise; `appendTo`, `slotArchetype`, `spec.extends`,
   `context.archetypes`, `context.extensions` and `context.start` each name a
   loaded `metadata.name`; `metadata.name` is unique across the loaded set;
   `extension` methods appear only in `type: extension` files; `openehr:
   "$reference"` is mandatory on a `reference` mapping; `mappingCode` names a
   registered function; every path resolves against the Web Template or the
   FHIR element table at load. A file that fails any of these is refused with a
   diagnostic carrying file, YAML path, mapping name and model path.

Rejected: a schema-derived AST (the schema is too defective to derive from,
and it would inherit the `manual` and `hierarchy` holes); validating against
the published schema alone (22.6% of the corpus would be refused, including
the model mapping the first round trip uses).

**Keyword casing is an adjudication, recorded on the tracker.** The library
spells `unidirectional` values four ways (21 of 50 uses are not the documented
`openehr->fhir` and `fhir->openehr`) and the first-milestone model mapping
writes `$openEHRRoot` four times where the specification writes
`$openehrRoot`. The specification's own text is inconsistent in the same way
(`openEHRCondition` beside `openehrCondition`, `openEHR:` beside `openehr:`),
so "spec-exact case" is undefined by the source. YAML keys stay exact, because
the schema fixes them. Keyword values (variables, `unidirectional` values) are
compared case-insensitively, a value outside the documented set is refused, and
a test asserts both spellings pass and a third is refused. This is
FerroBRIDGE's own decision on a specification contradiction, reported upstream
with the library fixes it implies.

### 4.3 Context resolution: one immutable program per (profile, template)

A context mapping plus its model mappings and extensions is compiled once into
an immutable program, validated in full at load, then interpreted per request.
The reference engine rebuilds its helper trees per composition and loads
extensions in database result order, so two runs can apply extensions in a
different order; nothing in the specification says which order is right. The
database literature (Kersten et al., PVLDB 2018, doi:10.14778/3275366.3284966)
finds neither compiled nor interpreted execution dominates, so the bridge takes
the separable win: lower once, interpret many.

**The ordering and collision rules, FerroBRIDGE's own where the specification
is silent:**

- Extensions apply in the order `context.extensions` declares them; within an
  extension, in file order. The compiler is a pure function of the file set.
- `add` appends to the model mapping; an `add` whose name collides with an
  existing mapping is a load error.
- `append` adds to the target's `followedBy`, which is the only thing the
  specification defines it to do ("To maintain readability the append is
  transformed into a followedBy"); an `append` carrying a `with` or a condition
  is a load error, never a silent replacement (the reference engine replaces a
  condition on append, which can widen a guard silently).
- `overwrite` replaces the mapping of that name; a missing name is a load
  error; two extensions overwriting the same name in one context is a load
  error.
- A duplicate `metadata.name` across the loaded set is a load error (the
  specification calls the name "a unique id"). The library's six files named
  `KDS_composition` cannot load together; the corpus test records that.
- `spec.extends`, `slotArchetype`, `appendTo` and `context.start` resolve by
  `metadata.name` only. There is no directory scope in the specification and
  the bridge invents none.
- The version selectors the specification lacks (which `metadata.version` of a
  name loads; `spec.openEhrConfig.revision` against the OPT; `profile.version`
  against the instance; `template.sem_ver` against the OPT) are a refusal at
  load when they disagree, as the first pass decided. A missing `sem_ver` or
  `profile.version` is accepted, because the schema makes them optional, and
  the program records "unpinned".

### 4.4 The interpreter: one engine, both directions

FHIRconnect declares its mappings bidirectional. Bidirectional transformation
theory gives the correctness property to build for: a lens is well behaved
when it satisfies GetPut (`put(source, get(source)) = source`) and PutGet
(`get(put(source, view)) = view`), and well-behaved lenses compose (Weber and
Ho, J Healthc Inform Res 2020, doi:10.1007/s41666-019-00065-0, applied to EMR
exchange). Two independently written direction functions are exactly the
duplicated logic that paper argues against.

**Decision:** one traversal engine with a direction parameter, over data-type
converters written once as lenses (`DV_QUANTITY` and `Quantity`,
`DV_CODED_TEXT` and `CodeableConcept`, `DV_DATE_TIME` and `dateTime` or
`Period`, the full matrix of the specification's data-type chapter). Direction
enters in three places only: conditions are evaluated on the input side only
(`fhirCondition` on FHIR to openEHR, `openehrCondition` on openEHR to FHIR, the
specification's rule); `unidirectional` markers skip a mapping in the other
direction; and the composition defaults apply inbound only. The data-type
matrix is where the cost is (the two value populators are 13% of the reference
engine), so it is table-driven and tested as a matrix, one case per cell per
direction.

The traversal rules, each pinned by the specification text or labelled as
ours:

- Top-down; a later mapping to the same `0..1` path overwrites an earlier one;
  a `0..n` path always appends; `0..n` into `0..1` takes the last element
  (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/recurrence/main.html>).
- `followedBy` concatenates the child path onto the parent's and iterates once
  per parent occurrence. Occurrences are structured indices in the program,
  never a regular expression over a stringified path.
- `manual` paths inside one entry merge into one element; a `manual` entry and
  a `with` mapping writing the same `0..1` node follow the top-down rule
  (ours; the specification scopes the merge guarantee to "inside manual
  mapping method").
- `../` and `^` are resolved at compile time against `$openehrRoot` and
  `$fhirRoot`. `^` is a prefix operator: `^` names the parent of the current
  FHIR anchor, `^^` its parent, and a `^` that would step above `$resource` is
  a load error. The reference engine does not implement `^` at all, so this is
  FerroBRIDGE's own pin of a one-sentence specification, and it is reported
  upstream as a request for a definition.
- `slotArchetype` recursion is cycle-checked over the whole chain, not one
  level.
- `hierarchy` `split` produces N resources from one archetype path; the
  deterministic id then includes the split occurrence (section 9).
- Type coercion is strict: a value that does not parse as the resolved RM leaf
  type or FHIR element type is a refusal, never a best-effort conversion (the
  specification delegates strictness to the vendor; this is the vendor's
  answer).
- PROGRAMMED mappings (`mappingCode`) are first-party Rust functions in a
  registry with a typed interface over the canonical model; an unknown code is
  a load error. The library depends on eight codes, six of them the Dosage and
  Timing gap the specification defers to its next version.

### 4.5 The FHIR path model

`with.fhir` is both read and written, and `^` and `$fhirRoot` are not
FHIRPath, so a FHIRPath evaluator cannot be the engine. No crate in the Rust
ecosystem offers a bidirectional path model over FHIR JSON; every FHIRPath
crate is evaluation-only and dependency-heavy (33 non-optional dependencies in
the one with a published compliance figure).

**Decision:** a hand-written path model over a JSON tree, guided by the element
table from `fhir-types`: navigation by element name, choice-type resolution
(`onset` with `ofType(Period)` becomes the `onsetPeriod` key), repeating
elements as arrays, primitive extensions in the `_element` sibling form
(<https://hl7.org/fhir/R4/json.html>). The three FHIRPath forms the mapping
library uses on the read side (`ofType()` and `as()`, `extension(url)`,
`resolve()`) are implemented as path-model operations. A general FHIRPath
evaluator is not adopted; if a real mapping corpus needs one, it goes behind a
cargo feature.

### 4.6 The facade

- **Facade, never store.** FerroBRIDGE exposes a FHIR R4 REST facade (create,
  update, read, transaction and batch Bundles with `If-None-Exist` and
  `If-Match` conditional forms, <https://hl7.org/fhir/R4/http.html>) and maps
  each request onto CDR operations. It stores no clinical data of its own. No
  prior art exists for this exact shape: the FHIRconnect reference engine
  exposes only `$tofhir` and `$toopenehr`, and the one bridge with a FHIR
  server surface embeds a FHIR store beside the CDR. The facade is chosen so a
  FHIR client sees one server.
- **Transaction is all-or-nothing, batch is per entry**, as R4 defines them. A
  transaction Bundle that cannot be mapped in full is refused with one
  `OperationOutcome` naming every failing entry and nothing is committed; a
  batch returns a per-entry outcome and counts failures.
- **Bundles are split by profile**, following the specification's engine
  chapter, with one correction: the chapter keys on `meta.url`, which does not
  exist in R4; the element is `meta.profile`, a list, so matching is set
  membership, not equality. A resource two mappings both need is a LINKED
  mapping. Unresolved references are fetched from the sending site with cycle
  protection; a reference that cannot be fetched is a refusal, because the
  chapter's "the engine proceeds with the mapping" contradicts its own
  strictness chapter and the strict reading wins.
- **Search is unspecified** by FHIRconnect ("does not focus specifically on
  AQL and FHIRsearch"). FerroBRIDGE's search is its own design, built after the
  round trips: an AQL projection per resource type declared beside the context
  mapping, executed over `POST /query/aql`, with the `CapabilityStatement`
  stating exactly which search parameters each resource supports and refusing
  the rest with an `OperationOutcome`. Labelled as FerroBRIDGE's extension
  wherever it appears.
- **Status codes are mapped as types**, from the ITS-REST outcome to the FHIR
  outcome, in a table that is FerroBRIDGE's own design with both sides cited
  and every row pinned by a wire test: a CDR `422` (template validation) is a
  facade `422` with the CDR's `validationErrors` in `issue.diagnostics`; a CDR
  `412` is a `412` with the current `ETag`; a CDR `404` on a deleted
  composition is a `410`; a CDR `401` propagates `WWW-Authenticate` and is
  never turned into a `403`; a CDR `405` or `415` is a facade `500`, because it
  means the bridge chose a call the CDR does not offer; any CDR `5xx` is a
  `502` carrying the upstream status, never an empty Bundle.

## 5. The OMOP side

OMOP is a relational analytics schema, not an API: the deliverable is rows in
a populated CDM v5.4 database. The reference engine is a pull-based batch ETL
over AQL, and the published literature on it (Kohler et al., J Biomed Inform
2023, doi:10.1016/j.jbi.2023.104437; Kohler et al., arXiv:2607.27208, 2026)
supplies the numbers that shape the design: 293 mapping entries over 196
archetypes, 91.5% of them landing in `MEASUREMENT` and `OBSERVATION`; 8.65% of
primary concept ids resolving to concept `0`, a figure the authors say
underestimates the gap; one diagnosis needing more than twenty linked records;
every required `FACT_RELATIONSHIP` carrying `relationship_concept_id = 0`
because no concept exists.

### 5.1 The ETL

- The OMOP engine is a batch ETL job runner over AQL (`POST /query/aql`)
  writing typed CDM rows into a PostgreSQL CDM database built from the OHDSI
  DDL, run as the `etl` subcommand of the one binary (section 7).
- **The unit of commit is the record graph of one composition.** A composition
  becomes many rows with `FACT_RELATIONSHIP` links between them; a partial
  write leaves links pointing at rows that were never written. Each
  composition's rows commit together or not at all, and a durable watermark per
  composition makes a resumed run exact. The reference engine commits batches
  of 1,000 rows as one transaction and a mid-run failure leaves a partial load
  with no marker.
- **Re-running is safe and produces the same state.** OMOP practice treats
  re-runs as the normal case (The Book of OHDSI, ETL chapter; Huser et al.,
  eGEMs 2016). The load replaces the rows of the compositions in the run by
  natural key and never appends blindly. The reference engine duplicates every
  clinical row on a re-run.
- **Results stream.** The AQL result set is paged and consumed as a stream, and
  rows reach PostgreSQL through binary `COPY` (`tokio-postgres`'s
  `BinaryCopyInWriter`), never a row-at-a-time insert. A national-scale
  FHIR-to-OMOP pipeline reports that a transformation engine which held the
  whole input in memory needed a chunking workaround at a billion resources
  (Essaid et al., JAMIA Open 2024, doi:10.1093/jamiaopen/ooae045).
- **Counted outcomes, never silent defaults.** Each run reports, per mapping
  and in total: rows written per table, `concept_id = 0` assignments,
  `relationship_concept_id = 0` links, source fields with no mapping, records
  refused and why. A required date is never synthesised; a record missing one
  is refused with a typed error naming the element (the 2026 paper names
  default-filled dates as a documented failure mode).
- An incremental mode has no specification behind it: ITS-REST 1.1.0 defines
  no change notification, subscription or bulk export (verified over the three
  `STABLE` OpenAPI documents). If FerroBRIDGE consumes a CDR's change events,
  that is FerroBRIDGE's own extension, labelled as such, and the batch path
  stays the conformant baseline.

### 5.2 The OMOCL interpreter

Every rule here fills a silence; OMOCL states none of them.

- **`alternatives` are tried in order and the first present value wins.** The
  library is consistent only with that reading (`measurement_date` lists the
  analyte's own date, then `../../`, then `../../../`; three `Person` columns
  end with `code: 0` as a terminal fallback). A required column with no matching
  alternative refuses the record. An alternative whose path matches several
  nodes produces one row per node when the entity iterates over that container,
  and is a refusal otherwise.
- **The OMOCL key to CDM column projection is specified by FerroBRIDGE, per
  RM type.** `value`, `unit`, `range_low`, `range_high` and
  `operator_concept_id` read the same `DV_QUANTITY` node and project
  `magnitude`, `units`, `normal_range.lower`, `normal_range.upper` and
  `magnitude_status` respectively (158 of 171 library cases alias them to one
  anchor); `concept_id` writes both `<x>_concept_id` (the standard concept) and
  `<x>_source_concept_id` (the source concept); a date key writes the `_date`
  column and, when the source carries a time, the `_datetime` column. The
  library writes `procedure_start_date` where the CDM column is
  `procedure_date`; the projection maps it and the mismatch is reported
  upstream.
- **`type` is the mapping author's declaration and the concept's domain is
  validated against it.** The CDM says the domain of the standard concept
  decides the table ("Write the data record into the table(s) corresponding to
  the domain of the Standard CONCEPT_ID(s)", <https://ohdsi.github.io/CommonDataModel/dataModelConventions.html>);
  OMOCL fixes the table in `type`. A literal `code` whose domain disagrees with
  `type` is a load error; a path-resolved concept whose domain disagrees is a
  refused record. The reference engine never reads `domain_id`. Routing by
  domain and treating `type` as a hint was rejected because it would silently
  move a record to a table the mapping author did not declare.
- **`Include`** resolves by `archetype_id` against the loaded set, is cycle
  checked, and may include the same archetype at two base paths (the
  laboratory result file does).
- **`CustomMapping`** names a first-party converter in a registry with a typed
  interface over the canonical model; an unknown name is a load error. The
  corpus names exactly one, `FactRelationshipCustomConverter`, in the file the
  first OMOP round trip loads, so that converter ships in v0.0.4.
- **`conceptMap`** is an at-code to `concept_id` table; a node whose at-code is
  not a key is a refused record. `multiplication` is a decimal product with
  overflow as a typed error.
- `person_id`, `visit_occurrence_id`, `*_type_concept_id`, `*_source_value`
  and `*_source_concept_id` appear in no OMOCL file. They are the engine's:
  `person_id` from the identity map (section 9), `visit_occurrence_id` from
  visit derivation, `*_source_value` from the openEHR path and code, and
  `*_type_concept_id` from deployment configuration with a documented default
  (the reference engine hard-codes `32817`; the CDM says the type concept
  records the provenance of the record, which a deployment knows and an engine
  does not).

### 5.3 Concept resolution

**OMOP concept resolution is not a FHIR terminology operation.** A source code
becomes a `concept_id` through the locally loaded OHDSI vocabulary: `CONCEPT`
by `(vocabulary_id, concept_code)`, `standard_concept = 'S'` or one
`CONCEPT_RELATIONSHIP` "Maps to" hop, with `invalid_reason IS NULL` and the
validity dates containing the record date, in a deterministic order; an
ambiguous match is a typed error, never the first row. Unmapped codes land as
`concept_id = 0` with the source value kept and the count reported. The
reference engine's query has the right shape and none of the filters (no
`invalid_reason`, no validity dates, no `ORDER BY`, `get(0)`), which is
reported upstream. This lives in the `omop-cdm` crate as SQL over the
vocabulary tables; the FHIR terminology client is a different component.

**The Athena export format is an open research item.** The CDM documentation
defines the ten vocabulary tables and says only "please visit athena.ohdsi.org"
about their distribution; the site is an authenticated application. The loader
is specified against the table definitions in the CDM CSVs, and the observed
file format of one export is pinned under a `PROVENANCE.md` labelled as
observed, never as a specification, before v0.0.4 is scheduled. The vocabulary
is a licence-gated deployment input: never in the image, a release asset, the
build context or CI.

### 5.4 Derived tables

`OBSERVATION_PERIOD` is explicitly the ETL's discretion in the CDM
(<https://ohdsi.github.io/CommonDataModel/ehrObsPeriods.html>, labelled
"suggestions"); `CONDITION_ERA` and `DRUG_ERA` have published SQL
(<https://ohdsi.github.io/CommonDataModel/sqlScripts.html>); `VISIT_OCCURRENCE`
in the reference engine comes from a configured AQL grouped by EHR and source
with `min(start)` and `max(end)`. FerroBRIDGE derives `OBSERVATION_PERIOD` from
the first and last clinical event per person, runs the published era SQL
verbatim, and takes visits from a configured AQL that is parsed and checked at
load with `openehr-query`. The visit type concept is configuration, never the
reference engine's hard-coded "inpatient". Each derivation is labelled as
FerroBRIDGE's own where the CDM gives discretion.

## 6. Terminology

The FHIR side needs three operations on a FHIR terminology server: `$lookup`
(the specification recommends resolving a code's display through a terminology
server because `DV_CODED_TEXT.value` is mandatory while FHIR `display` is not),
`$translate` for `conceptmap` references, and `$validate-code`. The server is
configured, never assumed; the client is version-tolerant across R4 and R4B;
the request and response contracts come from `fhir-types::r4::operations`. A
failed lookup is a typed error carrying the upstream status and body. Whether a
missing display then refuses the mapping or falls back to `coding.code` is a
deployment setting whose default is refusal, because the specification offers
the fallback as an "alternatively", not a rule. The OMOP side uses the local
vocabulary tables (section 5.3).

## 7. Workspace layout

The Business Source License 1.1 throughout for the project's own crates
(`LICENSE`); vendored upstream artefacts keep their own terms. The library
crates are published to crates.io (owner decision 2026-09-05), with a lockstep
crate version line beside the product version; the server and the tools are
not. Every name below was free on crates.io on 2026-09-05. "openEHR" is a
registered trademark of the openEHR Foundation and the crate descriptions say
so, as the published `openehr-*` crates do.

| Crate | Role | Kind | Published |
|---|---|---|---|
| `openehr-mapping-core` | the shared header model; the YAML loader (`serde-saphyr`: anchors, aliases, merge keys, source positions); the archetype-keyed mapping registry; the diagnostic model (file, YAML path, mapping name, model path); the RM-path model with `../` resolution | hand-written | yes |
| `openehr-path` | the `aqlPath` index over a Web Template with leaf RM type resolution; relative path derivation; composition build and read over `openehr-its` and `openehr-rm` | hand-written | yes |
| `fhir-types` | the FHIR model: per-version resources, datatypes and primitives with the strict JSON and XML codecs, the terminology operation contracts, the public element table; emitted by `tools/fhir-codegen` from the vendored HL7 packages | generated | yes (inherited line) |
| `fhir-tree` | the bidirectional path model over FHIR JSON guided by the `fhir-types` element table; choice types, repeating elements, primitive extensions; the three read-side FHIRPath forms | hand-written | yes |
| `fhirconnect-model` | the FHIRconnect AST; FerroBRIDGE's strict schemas; the published schemas vendored and exercised; semantic validation | hand-written | yes |
| `fhirconnect-resolve` | context resolution into one immutable program per (profile, template); extension ordering and collision rules | hand-written | yes |
| `fhirconnect-engine` | the bidirectional interpreter over `fhir-tree` and `openehr-path`; the data-type lens matrix; the PROGRAMMED registry | hand-written | yes |
| `omocl-model` | the OMOCL AST; FerroBRIDGE's authored JSON schema; the key-to-column projection tables; validation | hand-written | yes |
| `omocl-engine` | the one-directional interpreter emitting record graphs of typed CDM rows; the `CustomMapping` registry | hand-written | yes |
| `omop-cdm` | CDM v5.4 row types and column metadata generated from the OHDSI field definitions; the OHDSI PostgreSQL DDL vendored verbatim and embedded; the vocabulary loader and concept resolver; the derived-table runners; the `COPY` writer | generated plus hand-written | yes |
| `ferrobridge-openehr` | the ITS-REST client over `reqwest`, with the `openehr-its` data types, `backon` retry, typed outcomes per status | hand-written | yes |
| `ferrobridge-term` | the FHIR terminology client over `fhir-types` | hand-written | yes |
| `app/ferrobridge` | the one binary: `serve` (the FHIR facade and the ETL job API), `etl` (a batch run), `cdm init` (apply the DDL), `vocab load`, `mapping check`; thin `main.rs` over a `lib.rs`; the `redb` identity store | hand-written | no |
| `tools/fhir-codegen` | the FHIR generator moved from the sibling, with its `emit --check` drift gate and its vendored packages | hand-written | no |
| `tools/omop-cdm-codegen` | the CDM generator with its `emit --check` drift gate | hand-written | no |
| `tools/ferrobridge-testkit` | fixtures, the synthetic vocabulary, the CDR and terminology stubs (`wiremock`), the container harness (`testcontainers`); a path-only dev-dependency | hand-written | no |

**One binary.** The server and the batch ETL share the mapping crates, the
CDR client and the identity model, so they are one binary with subcommands.
The decisive reasons are operational: one exec-form `ENTRYPOINT` makes both
`serve` and `etl run` PID 1 with `SIGTERM` delivered directly, and a
Kubernetes `CronJob` or `Job` sets `args:` without overriding `command:`; with
two binaries a missing override silently starts a server in a Job that never
completes. It also halves the attestation and SBOM surface (section 12). If a
deployment wants a server image with no database client, that is a cargo
feature on the one crate, never a second binary.

**Dependencies, verified against crates.io on 2026-09-05** and recorded in
`docs/VERSIONS.md`: `openehr-base`, `openehr-rm`, `openehr-its`,
`openehr-query` 0.0.61; `serde-saphyr` 1.2.0 (the
maintained serde YAML with anchors, aliases, merge keys and spans;
`serde_yaml` is archived, `serde-yaml-ng` and `serde_yml` unmaintained);
`jsonschema` 0.53.0 with `default-features = false`; `axum` 0.8.9, `tower-http`
0.7.1, `reqwest` 0.13.4 with rustls, `backon` 1.6.0; `sqlx` 0.9.0 for checked
queries and `tokio-postgres` 0.7.18 for binary `COPY`; `redb` 4.2.0 for the
identity store; `jiff` 0.2.35 for the bridge's own timestamps (openEHR partial
dates stay in their lexical form in `openehr-base`; FHIR primitives keep theirs
in `fhir-types`); `sha2` 0.11.0; `insta`, `proptest`, `wiremock`,
`testcontainers` 0.27.3 for tests. Consuming `openehr-its` pulls in `axum`,
`moka`, `jsonschema` and `quick-xml` as hard dependencies, an accepted cost
recorded here. `openehr-adl`, `fhir-terminology` and every FHIRPath crate stay
out.

## 8. What each seam carries

- `openehr-mapping-core` to both model crates: a parsed header, a YAML value
  tree with positions, a registry lookup by `metadata.name` and by archetype
  id, a diagnostic.
- `openehr-path` to both engines: a resolved Web Template node (RM type,
  occurrences, `aqlPath`, node id), a canonical composition tree, path
  navigation and value read and write over it.
- `fhirconnect-resolve` to `fhirconnect-engine`: one immutable program, an
  `Arc`-shared value, with every path pre-resolved to a Web Template node or a
  FHIR element and every occurrence index structured.
- `fhirconnect-engine` to the facade: a composition tree plus a list of
  defaulted fields and warnings, or a FHIR resource set plus the same, or a
  typed refusal naming every failing element.
- `omocl-engine` to `omop-cdm`: a record graph of typed rows for one
  composition, with natural keys, plus counted outcomes.
- `ferrobridge-openehr` to everything above it: typed results per call, with
  every CDR status a variant carrying the upstream body; never an `Option` for
  a failure.
- `ferrobridge-term` to `fhirconnect-engine`: typed lookup, translate and
  validate outcomes, with a failed call a typed error.

Identifiers cross every seam as distinct newtypes: an `EhrId`, a
`VersionedObjectUid`, an `ObjectVersionId`, a `FhirResourceId`, a `PersonId`
are five types, and the derivation functions in section 9 are the only places
they meet.

## 9. Identity, failure and the specification's recommendations

The FHIRconnect engine chapter is explicitly "recommendations". FerroBRIDGE
pins each as its own decision, and the second pass tightened three of them.

- **FHIR identity.** The specification recommends a deterministic id as a hash
  of the composition UID and the entry path
  (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/engine/id-management.html>).
  Two facts pin the shape. FHIR R4 `Resource.id` is `[A-Za-z0-9\-\.]{1,64}` and
  "once assigned, this value never changes" (<https://hl7.org/fhir/R4/resource.html>).
  An openEHR `OBJECT_VERSION_ID` is `object_id::creating_system_id::version_tree_id`,
  and only the leading `object_id` (the `versioned_object_uid`) is stable
  across updates (RM Common §OBJECT_VERSION_ID). So the id is a SHA-256 over
  (`versioned_object_uid`, entry path, split occurrence), rendered as 52
  lowercase base32 characters with no padding, which fits the FHIR id grammar
  and stays stable across composition versions; `meta.versionId` carries the
  `version_tree_id`. A hash over the full version id would change the FHIR id
  on every update and break the FHIR rule. The specification's recommended key
  is not unique under a `hierarchy` split, so the split occurrence is part of
  the input. The id map (`redb`) records patient identifier to `ehr_id`,
  external to internal resource id, and internal resource id to composition
  uid, so a `PUT` resolves and a re-sent Bundle is recognised. The acknowledged
  failure mode (a re-sent Bundle omitting one resource reads as a different
  mapping) is documented, not hidden.
- **OMOP identity.** Every CDM v5.4 primary key is a 32-bit `integer`
  (`OMOP_CDMv5.4_Field_Level.csv`), so a content hash cannot be the surrogate
  key: the birthday bound puts a collision near 65,000 rows. The surrogate keys
  come from PostgreSQL sequences, and a bridge-owned side table in its own
  schema beside the CDM maps the natural key (`ehr_id`, `versioned_object_uid`,
  archetype path, occurrence) to the surrogate id and the load watermark. That
  table is what lets a re-run replace its earlier rows, and it keeps the
  openEHR identity in `*_source_value` as the OHDSI convention reserves it
  (The Book of OHDSI, ETL chapter). `person_id` maps one `ehr_id` to one
  person; reconciling one person across several EHRs is a deployment decision
  the CDM leaves to the ETL, and the bridge does not guess it. No specification
  governs this: our own design.
- **Failure policy.** Element-level failure is a typed error that fails the
  unit or is reported as a first-class issue; it is never a log line. FHIR:
  transaction all-or-nothing, batch per entry (section 4.6). OMOP: one
  composition's record graph all-or-nothing (section 5.1). Upstream: a refused
  call, a failed terminology lookup, a timeout, a `204` on a deleted
  composition, an ambiguous concept match are each a typed error carrying the
  upstream status and body, never a default, never an empty value (the
  reference OMOP engine turns a CDR outage into "no visits").
- **Type coercion** is strict (section 4.4).
- **PROGRAMMED mappings and `CustomMapping`** are named Rust functions
  registered at build time; there is no runtime plugin loading (sections 4.4,
  5.2).
- **Version mismatch** between mapping version, grammar version, archetype
  revision, template `sem_ver` and profile version is a refusal at load time
  (section 4.3).
- **Provenance.** Inbound, every defaulted or engine-set value is recorded in
  `FEEDER_AUDIT`; outbound, a `Provenance` resource with `entity.role =
  derivation` names the composition version (<https://hl7.org/fhir/R4/provenance.html>).
  The CDM has no provenance element, so the OMOP side records provenance in the
  bridge-owned side table only. No specification governs this: our own design.

## 10. The generated layer and the vendored inputs

Code is generated where a machine-readable source exists, hand-written
everywhere else, under `.claude/rules/codegen.md`.

**Generated by FerroBRIDGE, the FHIR model:** `fhir-types`, by `tools/fhir-codegen`
from the vendored `hl7.fhir.r4.core` 4.0.1, `hl7.fhir.r4b.core` 4.3.0,
`hl7.fhir.r5.core` 5.0.0, `hl7.fhir.r6.core` 6.0.0-ballot5 and `hl7.terminology`
7.3.0 packages (CC0), fetched from the FHIR package registry with checksum
verification. Root set per version: every `kind: resource` StructureDefinition
behind the `resources` feature, the terminology root set otherwise, each with
the complete closure of the datatypes and primitives it references, plus the
terminology `OperationDefinition`s. Output: the typed structs, the strict codecs
(<https://hl7.org/fhir/R4/json.html>), the element table, the operation
contracts; byte-deterministic; `emit --check` in CI.

**Generated by FerroBRIDGE, the CDM:** the `omop-cdm` row types and column metadata,
from `OMOP_CDMv5.4_Field_Level.csv` and `OMOP_CDMv5.4_Table_Level.csv` at tag
`v5.4.3`. Root set: every table in the definitions, all 39 across the `CDM`,
`VOCAB` and `RESULTS` schemas, emitted complete; a bridge writes a dozen of
them and reads ten, and the closure is emitted anyway, because a partial table
set is the kind of omission the rule forbids. Per field: name, CDM datatype
mapped to a Rust type (`integer` to `i32`, `float` to `f64`, `varchar(n)` to
a bounded string newtype, `date` and `datetime` to lexical newtypes), required,
primary key, foreign key. The one `Integer` (capital I) in the definitions is
normalised by the emitter with a `NOTE` and reported upstream. The emitter
iterates ordered structures and the output is byte-deterministic; `emit
--check` in CI fails on any diff. The DDL is not generated: OHDSI renders it
from the same CSVs through a dialect layer that sits outside them (SqlRender,
a hand-written index script), so the rendered PostgreSQL files
(`OMOPCDM_postgresql_5.4_ddl.sql`, `_primary_keys.sql`, `_indices.sql`,
`_constraints.sql`) are vendored verbatim and embedded, and a test asserts the
generated column set equals the DDL's.

**Generated elsewhere and consumed:** the openEHR RM, the ITS-REST types, the
OPT and Web Template codecs (`openehr-*`, from the BMM and the OpenAPI).

**Deliberately hand-written, because it is the product:** both mapping ASTs
(the published FHIRconnect schema is too defective to derive from, and OMOCL
has none), FerroBRIDGE's strict schemas for both languages, the compilers, the
interpreters, the data-type lens matrix, the path models, the clients, the
facade and the ETL runner.

**Vendored verbatim with provenance**, each by a committed
`scripts/vendor/*.sh` and each read by a test or a generator: the two
FHIRconnect schemas; the FHIRconnect mapping library; the OMOCL corpus; the
CDM CSV definitions and PostgreSQL DDL; the three `STABLE` ITS-REST OpenAPI
documents (read by the client's contract tests); the five HL7 packages the
FHIR generator reads.

## 11. Verification

- **The lens laws are the round-trip oracle.** GetPut over a golden corpus of
  compositions: openEHR to FHIR to openEHR reproduces the input modulo the set
  of fields the program declares unmapped or defaulted, and that set is asserted
  exactly, so the property is an equality. PutGet over a golden corpus of FHIR
  resources the same way. Both as `proptest` properties over generated
  instances of each data type, and as `insta` snapshots over the corpus.
- **Both mapping libraries are vendored and exercised in full.** Every file
  validates against the published schemas or is in the pinned rejection set;
  every file validates against FerroBRIDGE's schemas or carries an adjudicated
  skip; every context compiles or its failure is a recorded library defect.
  Coverage ratchets: cases are added, never removed.
- **The first FHIR round trip (v0.0.3):** the published
  `EVALUATION.problem_diagnosis.v1` model mapping verbatim, the published
  `KDS_problem_diagnose`, `KDS_problem_qualifier`, `KDS_lebensphase` and
  `KDS_anatomical_location` extensions verbatim, and a FerroBRIDGE-authored
  context and composition extension under the project's own directory, because
  the published `KDS_diagnose.context` cannot load strictly: its
  `KDS_composition.Condition` extension extends a name that does not exist and
  the `CLUSTER.lebensphase.v0` model file has a null `mappings`. Writing a
  project context is the specification's own customisation mechanism. An R4
  `Condition` is committed to a CDR over ITS-REST as canonical JSON, read
  back, mapped to FHIR again, and equal modulo the declared set; the library
  defects are reported upstream.
- **The first OMOP round trip (v0.0.4):** the published
  `Laboratory_test_analyte_v1` and `Laboratory_test_result_v1` files verbatim,
  emitting `MEASUREMENT` rows and the `FACT_RELATIONSHIP` rows of the one
  `CustomMapping` into a CDM v5.4 PostgreSQL database built from the vendored
  DDL, with a real Athena vocabulary loaded outside CI and the synthetic
  vocabulary fixture inside it, resolved `concept_id`s asserted, unmapped codes
  landing as `0` and counted, and a second run producing an identical database.
- **The OMOP acceptance gate** is the OHDSI Data Quality Dashboard's check set
  (Blacketer et al., JAMIA 2021, doi:10.1093/jamia/ocab132), run against the
  populated database outside CI, and a Rust port of its conformance and
  completeness checks that the bridge can run itself inside CI.
- **There is no external conformance suite for either language** and no second
  implementation of OMOCL, so FerroBRIDGE's corpus tests are the conformance
  instrument (issue #24) and a candidate outbound contribution.
- The composed test stack runs an openEHR CDR, reached over ITS-REST only, a
  FHIR terminology server and a PostgreSQL CDM; all three are configured
  deployments, never compile-time dependencies. The unit and integration
  layers stub the two servers with `wiremock`; only the end-to-end layer runs
  real ones.

## 12. Supply chain and release

No specification governs this: our own design, on the SLSA v1.2 build levels
(<https://slsa.dev/spec/v1.2/levels>) and GitHub's artifact-attestation guidance
(<https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations/using-artifact-attestations-and-reusable-workflows-to-achieve-slsa-v1-build-level-3>).
Issues #22 and #23 carry the contracts; the decisions that shape them:

- **Build Level 3 through reusable workflows.** The binary build and the image
  build are `uses:` calls to reusable workflows and nothing else, so no
  caller-defined step can reach the signing identity. Provenance and SBOM
  attestations are verified with `gh attestation verify --signer-workflow`.
- **One binary, one asset family per target:** the tarball, its checksum, its
  provenance and SBOM attestations, its `.intoto.jsonl`, its CycloneDX SBOM;
  four native targets; `compose.yaml` beside them. The finalize step refuses to
  publish an incomplete set, and immutable releases means there is no repair
  after publish.
- **The binary carries its own dependency list** (`cargo-auditable`), so
  `syft` can build the image SBOM from a distroless filesystem that holds one
  file; the image is built from the already-attested tarballs after verifying
  each.
- **The crates.io lane** publishes the twelve library crates in dependency
  order through Trusted Publishing, with a dry run on every pull request, a
  crate-version guard, and both drift gates (`fhir-codegen`, `omop-cdm-codegen`) ahead of the
  publish dry run so a published generated crate never disagrees with its
  generator.
- **The Athena vocabulary and every licence-gated input stay out** of the
  image, the release assets, the build context and CI (section 5.3).
- **The quickstart `compose.yaml`** adds a PostgreSQL CDM with a required
  password variable and a health-gated start, a `cdm-init` one-shot, a
  `vocab-load` profile over a read-only bind mount, and optional CDR and
  terminology server profiles pinned by digest.

## 13. Build order

Milestones are releases on the 0.0.x line. Each increment compiles, is tested,
and is green before the next starts. The tracker carries the issues; this is
the order and the reason for it.

**v0.0.2, the foundation.** The workspace with every lint (#20); the vendor
scripts and provenance for the corpora and the HL7 packages; `fhir-types` and
`fhir-codegen` moved in from the sibling and republished from here;
`openehr-mapping-core`; `omop-cdm` with its generator and drift gate (the
generated layer lands with the workspace); `openehr-path`; `ferrobridge-openehr`; `ferrobridge-term`;
the testkit; the server shape (#21); the container (#22); the release lane
with the crates leg (#23); the first publish of every crate at 0.1.0. Nothing
maps yet; everything the mapping needs exists and is published.

**v0.0.3, the FHIR round trip.** `fhir-tree`; `fhirconnect-model`;
`fhirconnect-resolve`; `fhirconnect-engine` with the data-type lens matrix for
the types the round trip touches; the facade's create, read, update and
transaction; the identity store; the round trip of section 11. Depends on the
`resources` feature of `fhir-types` (v0.0.2).

**v0.0.4, the OMOP round trip.** `omocl-model` with the authored schema;
`omocl-engine`; the vocabulary loader after the Athena format is pinned; the
concept resolver; the `COPY` writer and the record-graph commit; the identity
side table; `FactRelationshipCustomConverter`; `OBSERVATION_PERIOD`, the era
SQL and AQL visits; the `etl` and `cdm init` subcommands; the conformance gate
over both corpora (#24); the round trip of section 11.

**v0.0.5, FHIR breadth.** The full data-type matrix; `reference`, `hierarchy`
split, LINKED and bundle splitting by `meta.profile`; reference fetching with
cycle protection; batch; the PROGRAMMED registry with the eight library codes;
terminology display resolution; `CapabilityStatement`; the full library
compiles or every failure is a recorded library defect.

**v0.0.6, OMOP breadth.** The remaining eight OMOCL targets; the full
key-to-column projection; domain validation over the whole library; the run
report; the Rust port of the Data Quality Dashboard checks; the whole corpus
loads or every failure is a recorded defect.

**v0.0.7, search.** FHIR search over AQL projections declared beside the
context mapping, the `CapabilityStatement` declaring exactly what is
supported, labelled as FerroBRIDGE's extension.

## 14. Decision register

| Decision | Choice | Ground | Rejected |
|---|---|---|---|
| Engine shape | one foundation, two interpreters, two sinks | the two grammars share a header and nothing structural (section 1) | one intermediate representation over both |
| FHIR model | `fhir-types` and `fhir-codegen` move into this repository; the bridge generates the shared FHIR model | the bridge is the widest consumer and drives the crate's evolution; one generator across both products; no crate flows the other way | widening in place in the sibling, a separate repository, `fhir` 4.2.2 (risk profile, code quality), `fhir-model`, `helios-fhir` |
| Mapping AST | hand-written types; published schema exercised; own strict schema; semantic validation | the published schema rejects 3 of 8 mapping types and 24 of 106 files (section 4.2) | schema-derived AST; published schema as the validator |
| Keyword casing | keys exact; keyword values case-insensitive within the documented set | the specification's own text is case-inconsistent; recorded and reported | refuse (breaks the first-milestone file); accept anything |
| Mapping execution | compile once into an immutable program, interpret per record | Kersten et al. 2018; the reference engine's per-request rebuild and unordered extensions | interpret the YAML tree per request; code generation per mapping |
| Extension order | declaration order, collisions are load errors | specification silent; reference engine nondeterministic (section 4.3) | last-writer-wins |
| Direction | one engine, lens converters, direction enters at conditions, `unidirectional`, defaults | Weber and Ho 2020; the specification's input-side rule | two engines; two converter sets |
| FHIR path handling | own bidirectional path model over JSON with the element table | `with.fhir` is written; `^` is not FHIRPath; no crate writes | a FHIRPath evaluator (all read-only, heavy) |
| openEHR wire | canonical JSON; Web Template built locally from the OPT | canonical is mandatory in ITS-REST; `WebTemplate` does not deserialise; node ids are server-specific | FLAT on the wire; fetching `wt+json` |
| FHIR identity | SHA-256 over (`versioned_object_uid`, path, split occurrence), base32, plus a recorded map | FHIR id grammar and immutability; the version id changes per update | hash of the full version id; random ids (no round-trip test possible) |
| OMOP identity | sequences plus a bridge-owned natural-key side table | every CDM 5.4 key is a 32-bit integer | content-hash surrogate keys (collide near 65k rows) |
| OMOP commit unit | one composition's record graph, all-or-nothing, with a watermark | more than twenty linked records per diagnosis (arXiv:2607.27208) | row batches |
| OMOP table routing | `type` is the declaration, the concept's domain is validated against it | the CDM's domain rule; the reference engine ignores it | route by domain; trust `type` blind |
| `alternatives` | ordered first-match | the only reading the library is consistent with | all-match; merge |
| One binary or two | one binary with subcommands | PID 1 and `args:` in Kubernetes; half the attestation surface | a server and an ETL binary |
| Crates | published, lockstep line, Trusted Publishing | owner decision 2026-09-05 | unpublished |
| YAML parser | `serde-saphyr` 1.2.0 | anchors, aliases, merge keys, spans, maintained | `serde_yaml` (archived), `serde-yaml-ng`, `serde_yml` |
| Bulk load | binary `COPY` via `tokio-postgres`, `sqlx` for checked queries | throughput; the reference engine's per-row persist is its ceiling | ORM-style inserts |

## 15. What is deliberately outside, and what is reported upstream

Outside: demographics (FHIRconnect sends resources to an unspecified external
endpoint, and the ITS-REST Demographic API is `DEVELOPMENT`), FHIR
Subscriptions (never mentioned), a materialised FHIR store, OMOP CDM versions
other than 5.4, and OMOP to openEHR (OMOCL has no construct for it). Each is a
tracker issue, not silence.

Reported upstream as `upstream-report` issues, each with the citation and the
resolution sought: the FHIRconnect schema defects and the three schema-invalid
mapping types; the `meta.url` element that does not exist in R4; the `../`
claim about openEHR; the `^` operator's one-sentence definition; the FHIRconnect
library's duplicate names, dangling references and mis-cased keywords; the
OMOCL grammar images that document a header no file uses and a
`ProcedureOccurrence` table naming condition columns; the CDM `Integer`
datatype typo; the ITS-REST OpenAPI's missing commit headers and contradictory
error schemas; the reference OMOP engine's concept resolution without validity
filters.
