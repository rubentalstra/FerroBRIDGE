# omocl

The OMOCL half of the bridge, hand-written over the shared foundation in
`openehr-mapping-core` and the CDM v5.4 metadata in `omop-cdm`. OMOCL
(<https://github.com/SevKohler/OMOCL>) and the OMOP Common Data Model are the
oracles; Eos is prior art.

## The grammar is thin, so every silence is labelled

OMOCL publishes two railroad images and four syntax tables
(`docs/specs/omocl/docs/wiki-images/`, the wiki's Syntax-and-grammar page) and
no schema. Read them before the library: the library is evidence of what real
files write, never an oracle. A form the library writes that no artefact
documents is recorded on the upstream report (#102) and, where the model
admits it, carries a `// NOTE:` naming it FerroBRIDGE's own design.

## Three layers, one verdict

`model::parse` lowers the positioned tree and is the layer that names a key
and its position; `model::schema` validates the same document against the
authored `schemas/omocl-mapping.schema.json`; `model::semantic` holds what a
schema cannot express. The loader runs them in that order and returns every
diagnostic. The schema's per-target key sets must equal the projection table,
and a test asserts it: change both together.

## The projection is data, and the CDM metadata is its check

`model::projection` is the one table from an OMOCL key to the CDM columns it
writes and the part of the value each column takes. A key it does not list is
a load error. Every column it names must exist in `omop_cdm::meta::table`,
which is generated from the OHDSI field definitions; the tests assert that for
the whole table. A fact about a CDM column (its domain, whether it is required)
is read from `omop_cdm::meta`, never restated here; a missing fact is an
emitter change in `tools/omop-cdm-codegen` (`.claude/rules/codegen.md`).

A key the library writes that no CDM column receives (`qualifier` under
`Measurement`) is listed in `KEYS_WITHOUT_COLUMN` and refused with its reason:
writing it nowhere would drop clinical content with no trace.

## The domain check takes the vocabulary from its caller

`semantic::check_concept_domains` takes a `domain_of` closure, so the concept
resolver of `omop-cdm` plugs in without this crate reading a database. A
literal concept id must exist in the vocabulary and, where the field
definitions name a domain for its column, belong to it. Concept `0` is exempt.

## Compile once, walk without parsing

`resolve::compile` turns a set into one immutable `Program` per template:
every path becomes a `Hop` (parent steps, then segments) checked against the
Web Template, so `engine` parses nothing. A part the template carries no node
for is recorded in `Program::unbound`, never dropped; a required column with
no bindable alternative, a cycle, a path variable or a positional step
refuses the compile. Keep structural faults in `resolve` and per-record
faults in `engine`.

## The engine refuses records, and fails runs only upstream

`engine::run` runs three passes: `walk` chooses each column's value per
record instance, the run asks the `ConceptSource` each distinct question
once, and `finish` projects into `omop_cdm::graph` rows (a cell map checked
against the CDM metadata). Values are decoded into the generated
`openehr-rm` types (`DataValue`, `Element`) and dates read with
`openehr-base`'s ISO 8601 types; a class or attribute fact comes from
`openehr_rm::v1_2::model`. Never walk canonical JSON by attribute name or
restate an RM shape here. `omop-cdm` is taken without its `database`
feature; the vocabulary question and answer are `omop_cdm::vocabulary`'s
`SourceKey` and `Resolution`, which need none. A primary source code mapped
to several standard concepts writes one row per concept, told apart by the
`RecordKey` discriminator's branch.
A record that cannot be written is a typed `RecordRefusal` in the outcome
and a `Refusal` in the graph's report, and the rest runs. A failed vocabulary call, a program of another template or a
missing domain concept is an `EngineError`: never turn one into concept `0`.
Concept `0` is written only where the CDM field definitions say so (no
standard concept, units given but unmapped); the writer counts them. Each
silence the engine fills carries a `// NOTE:` or a doc line naming it our own
design; the graph seam is `omop_cdm::graph`, owned by `omop-cdm`.

## Tests

`tests/it/corpus.rs` pins the exact set of library files FerroBRIDGE refuses,
each with its first diagnostic and its defect, the eight `metadata.name`
mismatches as data, and the per-type key union as a committed snapshot. A file
that starts or stops failing is re-adjudicated against the grammar, never
absorbed. `tests/it/resolve.rs` and `tests/it/engine/` run over the
synthetic laboratory template (`ferrobridge_testkit::fixtures::LABORATORY_REPORT_OPT`)
and its FLAT composition (`LABORATORY_REPORT_FLAT`), with a map-backed
`ConceptSource` in `tests/it/lab.rs`; each engine rule has its own case and
the laboratory files end to end are two committed snapshots. Fixtures are
synthetic content invented for the test.
