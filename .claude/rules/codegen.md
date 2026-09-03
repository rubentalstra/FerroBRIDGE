<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Code generation: the intended discipline, pending the research

**Nothing is generated yet, because no code exists.** This file is the rule
that applies the moment a generated layer lands, written now so the decision is
made against a standing discipline rather than in the middle of an
implementation. The boundary itself is a research question on issue #1
(`CLAUDE.md` §The two layers).

## Why a generated layer is the expected shape

Both sibling projects generate their specification model rather than
hand-writing it, for the same reason: the specification bodies publish the
model in machine-readable form, and a hand-transcribed copy of a large model
drifts from its source with no way to detect it. HL7 publishes the FHIR type
system and every operation as `StructureDefinition` and `OperationDefinition`
resources in versioned packages, which is what FerroTERM generates its FHIR
crate from. Whether FerroBRIDGE needs the same layer, consumes a published
crate, or needs neither is what the research settles, and the same question is
open for the openEHR side and the OMOP side.

## The rules, once anything is generated

- **Never hand-edit a `// @generated` file.** Every generated file starts with
  a `// @generated … DO NOT EDIT` banner. To change output, edit the emitter or
  its override map, then regenerate. A doc defect, a wrong field type, or a
  missing variant in a generated file is a generator fix plus regeneration,
  never an edit of the output.
- **Fix the emitter, never the consumer.** When engine code hits a shape in a
  generated crate that is wrong or insufficient versus the vendored input (a
  missing field, a type too narrow, a per-version difference absent), the fix
  is an emitter or override change plus regeneration. A shadow type, a
  duplicate model, an adapter layer, a placeholder value, or a "temporary"
  local representation in the consumer is forbidden: it silently forks the
  model and defeats the whole design. If the emitter fix is large, register a
  tracker issue; the workaround is still forbidden. On discovering an existing
  workaround, register its removal.
- **The emission scope is a DECLARED root-set closure, emitted COMPLETE.** A
  bridge does not need every resource or table a specification defines, so the
  generator declares its root set and emits the COMPLETE transitive closure of
  everything those roots reference, at its source-mirrored location. Within
  that closure, completeness is absolute: never narrow a schema merge, prune a
  referenced type, or suppress a "missing" generated file to quiet a diff or
  dodge a build error. That is hiding code that should exist. Widening or
  narrowing the root SET is a deliberate, recorded decision, never an ad-hoc
  per-file omission.
- **Per-version correctness is by construction.** Where a specification has
  versions, the generator emits per version from the machine-readable input, so
  a version difference is a generated difference rather than a hand-written
  conditional that can drift. If two versions genuinely coincide the emitter
  may share, and the decision is the emitter's, driven by its inputs.
- **The output is byte-deterministic.** The emitter iterates ordered structures
  (`BTreeMap`, sorted vectors), so regeneration with unchanged inputs produces
  an identical tree. That is what makes a drift check meaningful
  (`reliability.md` §Determinism).
- **A drift check regenerates in CI and fails on any diff**, so the generated
  tree is always in sync with the vendored inputs and the current emitter. The
  check lands in the same change as the generator, never later.
- **A generated file is marked as generated to git too**: the
  `.gitattributes` entry (`linguist-generated`) collapses it in diffs and keeps
  it out of language statistics.
- **The inputs are vendored verbatim with provenance** and are never hand
  edited (`vendored-inputs.md`).

## What is never generated

The bridge engine: the mapping loader and validator, the mapping interpreter,
the clients for the CDR and the terminology server, and the server surface.
That is hand-written idiomatic Rust of our own design (`rust-style.md`),
consuming the generated types directly, and it is the product.
