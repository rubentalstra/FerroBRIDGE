# fhir-types

The generated FHIR layer. Every `.rs` file here is produced by
`tools/fhir-codegen` from the vendored FHIR packages and is off-limits
to hand edits; the only hand-maintained files are `Cargo.toml` and this
`CLAUDE.md`. Full discipline: `.claude/rules/codegen.md`.

- To change anything under `src/`, change the emitter or its inputs, then run
  `cargo run -p fhir-codegen -- emit` and commit the regenerated tree.
  `cargo run -p fhir-codegen -- emit --check` is the CI drift check and
  fails on any difference.
- The emission scope is per feature, over one emitted tree. `terminology` is
  the first declared root set (Bundle, CapabilityStatement, CodeSystem,
  ConceptMap, OperationOutcome, Parameters, TerminologyCapabilities, ValueSet,
  and the terminology OperationDefinitions); `resources` widens it to every
  concrete `kind: resource` StructureDefinition of the package. Each carries
  the complete closure of every type its roots reference, per version, and each
  emitted item carries the `cfg` of the narrowest feature that selects it.
  Never trim inside a closure.
- The four version modules are behind `r4`, `r4b`, `r5` and `r6`; the default
  feature set is every version plus `terminology`, which is the surface the
  sibling terminology server consumes.
- The operation contracts stay the terminology set.
- `schema::SCHEMAS` is the one element table per version: the XML codec and the
  path model read the same statics, so cardinality, the element path, the type
  codes and `contentReference` are never copied into a second table. The
  conversion to `serde_json::Value` at the HTTP edge is #122.
- `Resource`-typed elements (`Bundle.entry.resource`, `contained`) hold the
  `Resource` enum over the root set plus `UnknownResource`, which keeps any
  other resource's JSON body so a Bundle round-trips.
- `doctest = false` is deliberate: generated doc text is not a curated
  example set. The crate-level `#![allow]` list in the generated `lib.rs`
  names the pedantic lints the specification's own text and shapes trip.
- The crate is Apache-2.0, the one first-party crate under a licence other
  than BUSL-1.1 (`docs/architecture.md` §4.1); `scripts/checks/versions.sh`
  carries the exception.
