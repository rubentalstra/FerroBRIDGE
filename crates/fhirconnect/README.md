<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# fhirconnect

The FHIRconnect mapping language for Rust: the mapping file model and its validation, context resolution into an immutable program, and the bidirectional interpreter between openEHR compositions and FHIR resources.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

Version 0.0.0 holds the crate name on crates.io until the first publish. The
crate is built module by module under
[FerroBRIDGE issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1);
the design is recorded in the repository's architecture document.

## The model module

`fhirconnect::model` is the mapping file model and the three layers that
validate it. `ast` carries one positioned Rust type per construct a model,
extension or context file may hold; `parse` lowers the positioned YAML tree the
shared loader in `openehr-mapping-core` produces, refusing an unknown key, a
node of the wrong kind and a keyword value outside its documented set (keyword
values compare case-insensitively, YAML keys are exact); `schema` validates the
same document against the two schemas FHIRconnect publishes and against the
stricter pair this crate ships under `schemas/`; `semantic` checks what a schema
cannot express, such as a `targetRoot` that is not a child of its `with` path or
a cross-file reference that names no loaded mapping; `load` runs all of it over
one file or a whole set and returns every diagnostic rather than the first. The
vendored mapping library is exercised by tests that pin the exact set the
published schemas refuse, so an upstream schema fix fails a test here.

## The tree module

`fhirconnect::tree` is the bidirectional path model behind `with.fhir`. A
mapping runs both ways through one expression, and two of the forms FHIRconnect
writes are not FHIRPath at all (`$fhirRoot` and the `^` parent operator), so the
module parses the expression itself, resolves it against the element table
`fhir-types` emits per FHIR version, and evaluates it over that crate's lexical
`Value` tree.

- **The element table is the authority.** Navigation is by element name, a
  choice element resolves through `ofType()`, `as()` or the suffixed name, a
  repeating element is an array indexed by a structured occurrence, and a
  primitive's `extension` lives in the sibling member named with a leading
  underscore ([FHIR R4 JSON](https://hl7.org/fhir/R4/json.html)).
- **Every expression is classified when it is parsed.** `where()`, `first()`,
  `last()`, an index filter and `resolve()` select among the values a document
  already holds, so an expression carrying one is read-only and a write through
  it is refused with the offending step named.
- **`resolve()` fetches nothing.** It returns a deferred outcome carrying the
  reference and the steps still to apply, which the engine finishes.
- **A write is all or nothing.** It applies to a copy and replaces the document
  only when every step succeeded.

## The resolve module

`fhirconnect::resolve` compiles one context mapping into one immutable program,
once, at load. It picks the start model mapping, applies the extensions the
context declares, checks the four version selectors, and pre-resolves every
path on both sides: the FHIR side against the element table with its
writability, the openEHR side to a node of the Web Template with the repeating
nodes above it as structured occurrence axes. The interpreter runs that program
per request and parses no path.

- **The program never changes after it is built.** Its fields are private, no
  method takes `&mut self`, and it travels behind an `Arc`.
- **The rules the specification leaves open are refusals, not guesses.**
  Extensions apply in declaration order and, within one file, in file order; an
  `add` whose name collides, an `append` carrying mapping logic, an `overwrite`
  of a missing method and two extensions overwriting one name each refuse with
  both mappings named.
- **A version disagreement is a refusal and a missing selector is recorded.**
  `metadata.version`, `openEhrConfig.revision`, `profile.version` and
  `template.sem_ver` are each checked against what the files and the template
  carry, and an optional selector nothing pins reads as unpinned.
- **A program is selected by identity.** The FHIR side matches the profile set
  an instance claims in `meta.profile`, the openEHR side the template a
  composition names, and an ambiguous selection with nothing pinning it refuses
  by naming the candidates.

## The engine module

`fhirconnect::engine` interprets a compiled program, and there is one traversal
for both directions. The direction enters it in three places only: a condition
is evaluated on the input side, a mapping whose `unidirectional` names the
other direction is skipped, and the composition defaults apply going into
openEHR.

- **A data-type cell is written once as a lens.** `get` reads an openEHR
  reference-model value into its FHIR element and `put` writes it back with the
  openEHR value the target already holds, and the pair is tested against the
  GetPut and PutGet laws as properties. The cells the FHIR round trip needs are
  in: `DV_CODED_TEXT` against `CodeableConcept` and against `Coding`,
  `CODE_PHRASE` against `Coding`, `TERM_MAPPING` against `Coding`, `DV_TEXT`
  against `string`, `Coding` and `CodeableConcept`, `DV_DATE_TIME` against
  `dateTime` and the one-way collapse of a `Period`,
  `DV_INTERVAL<DV_DATE_TIME>` against `Period`, `PARTY_IDENTIFIED` against
  `Reference` with `DV_IDENTIFIER` against `Identifier` beside it, and
  `DV_PROPORTION` against `Quantity` for a percentage.
- **A date and time value keeps the text it came with.** Both sides hold a
  lexical string, so `Z`, `+00:00`, `+01:00` and fractional seconds survive;
  the engine parses no timestamp and adds no offset.
- **An attribute with no counterpart is carried, a falsified value refuses.**
  `put` restores the attributes the data-type table marks `-` from the openEHR
  value the target holds; a `TERM_MAPPING` whose `match` is not `=` asserts an
  equivalence the data denies and is refused instead.
- **A `DV_PROPORTION` that is not a percentage refuses.** FHIR `Quantity`
  carries no denominator, and the engine writes no extension URL of its own for
  one.
- **A loss is a typed outcome, never a log line.** The declared set is a
  `unidirectional` skip, a defaulted composition field, the occurrences a
  `0..n` into a `0..1` dropped, a one-way data-type row, and a reference left
  for the facade to resolve. Anything else refuses the unit.
- **An occurrence is a structured index.** One entry per repeating element on
  the way to the target, so a `0..1` output is overwritten, a `0..n` output is
  appended to, and a child mapping writes under the occurrence its parent is
  bound to.
- **One walk runs the program, from either side.** `to_openehr` reads a FHIR
  resource and builds a composition through the Simplified Formats seam;
  `to_fhir` reads a composition and writes the resource the context names. Both
  run the same mapping list top-down: the input-side conditions, the input
  occurrences, the data-type cell, the write, then the `followedBy` children
  once per occurrence the parent bound. A `manual` entry merges every path it
  names into one element, a `slotArchetype` recurses with the whole chain
  checked for a cycle, and `type: NONE` writes nothing and only anchors what
  follows.
- **A missing required child refuses.** A `followedBy` child whose openEHR node
  the template constrains to `1..1` and whose input carries nothing is
  `EngineError::MissingRequired`, naming the node.
- **The composition defaults apply going into openEHR only.** The composer and
  the context start time carry the values the specification's own defaults
  chapter suggests, and the language and territory are the caller's, because
  the chapter assigns them to the project. Each one the engine fills is a
  recorded loss.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
