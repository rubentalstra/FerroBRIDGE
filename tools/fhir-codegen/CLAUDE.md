# fhir-codegen

The generator: vendored FHIR packages in, `crates/fhir-types` out, and the
fetched HL7 v2 definitions in, `crates/hl7v2-types` out. Hand-written
tooling; the vendored `StructureDefinition` and `OperationDefinition`
resources are the authority for what it emits (`.claude/rules/codegen.md`).

- Inputs live under `vendor/<package>/`, fetched only by
  `scripts/vendor/fhir-packages.sh`, verbatim, with a `PROVENANCE.md` each
  (`.claude/rules/vendored-inputs.md`). Never hand-edit a vendored file.
- The pipeline is `package` (read), `snapshot` (resolve), `roots` (select),
  `closure` (the root-set closure), `lower` (the Rust model: structs per
  type and backbone element, enums per choice, boxed cycle edges), `render`
  (source text), `emit` (write or `--check`). Two root sets are declared,
  `roots::RootScope::Terminology` and `roots::RootScope::Resources`; the
  emitter emits one tree holding the union with the complete closure of the
  types each references, per version, and marks every item with the `cfg` of
  the narrowest feature that selects it. A shape the consumer lacks is fixed
  here, never shadowed downstream.
- The `v2` root set (`src/v2/`, #253) reads the HL7 v2 definitions that
  `scripts/vendor/v2ig.sh` fetches into the ignored `vendor/hl7-v2ig/` and
  emits `crates/hl7v2-types`: `v2::corpus` (the manifest-less loader over the
  directories `roots` declares), `roots::V2RootSet` (every message structure and segment),
  `v2::lower` (the differential read as the snapshot, the position-prefixed
  id rule, the extensions, the segments the structures reach), `v2::render` and `v2::emit`.
  `v2::definition` is the v2 files' own serde projection, refusing every
  member it does not name, so the FHIR projection in `fhir.rs` stays strict.
  A defect of the definitions is tolerated only in the files
  `v2::lower::Defect::tolerated_in` lists, and a test asserts each listed
  file carries it. `emit` and `emit --check` cover both crates, and both need
  the fetched tree.
- A primitive whose JSON form is a string carries its lexical form into the
  output: `lower` reads the `regex` extension of `<primitive>.value` from the
  version's own package and compiles it there, so an uncompilable form fails
  the emit rather than the crate, and `render_codec` writes it anchored behind
  a `LazyLock` with the function that refuses a value outside it
  (<https://hl7.org/fhir/R5/datatypes.html#primitive>). The forms are XML
  Schema patterns, so both sides compile them with Unicode mode off
  (`lower::compile_lexical_form`), and they differ between versions, so each is
  read from its own package and never copied.
- Output is byte-deterministic: iterate `BTreeMap` and sorted vectors, never
  a hash map (`.claude/rules/reliability.md`); `rustfmt` from the pinned
  toolchain formats the output so `cargo fmt --check` and the emitter agree.
- `cargo run -p fhir-codegen -- emit` regenerates; `-- emit --check`
  is the drift check CI runs.
- `main.rs` is thin over `lib.rs` so the loader and emitter are tested
  through the library.
- This crate is a tool, so it may write to stdout and stderr; every such site
  carries a scoped `#[expect]` with a reason.

## Naming

Names say what a thing is in its domain, never which language or pipeline
stage produced it. The FHIR side keeps FHIR's names verbatim
(`fhir::StructureDefinition`, `fhir::ElementDefinition`,
`snapshot::ResolvedStructure`); the generated side names the artefact
(`lower::VersionModule` is the generated module for one FHIR version,
`lower::TypeDef` one type definition, `lower::Cardinality` a cardinality). A
`Rust`, `Fhir`, or `Gen` prefix on a type name is a smell: if two views of one
thing collide, the module path disambiguates (`fhir::` versus `lower::`).
