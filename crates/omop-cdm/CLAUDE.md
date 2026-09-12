# omop-cdm

The OMOP Common Data Model v5.4 layer. `src/generated/` is produced by
`tools/omop-cdm-codegen` from the vendored OHDSI definitions and is off-limits
to hand edits; `src/lib.rs`, `src/meta.rs`, `src/value.rs`, `src/ddl.rs`,
`Cargo.toml`, `tests/` and this `CLAUDE.md` are hand-written. Full discipline:
`.claude/rules/codegen.md`.

- To change anything under `src/generated/` or `ddl/`, change the emitter or
  its inputs, then run `cargo run -p omop-cdm-codegen -- emit` and commit the
  regenerated tree. `cargo run -p omop-cdm-codegen -- emit --check` is the CI
  drift check and fails on any difference.
- The emission scope is every table of the definitions, all 39 across the
  `CDM`, `VOCAB` and `RESULTS` schemas, each with every column in definition
  order. Never trim it: a bridge writes a dozen of these tables and reads ten,
  and the rest are emitted anyway.
- The four PostgreSQL DDL files under `ddl/` are OHDSI's rendered output,
  copied verbatim into the crate by the emitter because `include_str!` has to
  reach them in a packaged crate (`ddl/PROVENANCE.md`). They are not generated:
  OHDSI renders them from the same definitions through a dialect layer that
  sits outside them.
- The hand-written half is the vocabulary the generated half is written in:
  `meta` for the column metadata, `value` for the three column types Rust
  has no type for (`Varchar<N>`, `CdmDate`, `CdmDatetime`), and `ddl` for the
  `@cdmDatabaseSchema` substitution. A shape the generated code needs is added
  here and emitted, never shadowed in a consumer.
- A date and a datetime are kept lexically and validated at construction. The
  crate depends on no time library: the bridge moves text between two systems
  and does no calendar arithmetic on it.
- `version` stays at the 0.0.0 crates.io name reservation until the crate line
  starts (`docs/VERSIONS.md`, `.claude/rules/crates-publishing.md`).
- The vocabulary loader and the concept resolver land in a later increment;
  they are SQL over the loaded OHDSI vocabulary, never a FHIR terminology
  operation.
