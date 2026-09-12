# omop-cdm-codegen

The generator: the vendored OMOP CDM v5.4 definitions in,
`crates/omop-cdm/src/generated/` out. Hand-written tooling; the OHDSI
`CommonDataModel` definitions are the authority for what it emits
(`.claude/rules/codegen.md`).

- Inputs live under `docs/specs/omop-cdm/`, fetched only by
  `scripts/vendor/omop-cdm.sh`, verbatim, with a `PROVENANCE.md`
  (`.claude/rules/vendored-inputs.md`). Never hand-edit a vendored file.
- The pipeline is `definitions` (read the two CSV files), `lower` (the Rust
  model: one table per module, one column per field, the CDM datatype mapped to
  its Rust type), `render` (source text) and `emit` (write or `--check`). The
  emission scope is every table the definitions carry, all 39 across the `CDM`,
  `VOCAB` and `RESULTS` schemas, with every column. A shape the consumer lacks
  is fixed here, never shadowed downstream.
- The four rendered PostgreSQL DDL files are copied into `crates/omop-cdm/ddl/`
  by `emit`, with the provenance note beside them, so `include_str!` reaches
  them in a packaged crate. The DDL is not generated: OHDSI renders it from the
  same definitions through a dialect layer that sits outside them.
- The definitions carry one field spelled `Integer` where every other integer
  column is `integer`, and one column name quoted as `"offset"` because OFFSET
  is a reserved word in SQL. The emitter normalises both, each with a `NOTE` at
  the site; the datatype typo is reported upstream on #103.
- Output is byte-deterministic: iterate `BTreeMap` and sorted vectors, never a
  hash map (`.claude/rules/reliability.md`); `rustfmt` from the pinned
  toolchain formats the output so `cargo fmt --check` and the emitter agree.
- `cargo run -p omop-cdm-codegen -- emit` regenerates; `-- emit --check` is the
  drift check CI runs as the `omop-cdm-codegen-drift` job.
- `main.rs` is thin over `lib.rs` so the loader and the emitter are tested
  through the library.
- This crate is a tool, so it may write to stdout; the one such site carries a
  scoped `#[expect]` with a reason.

## Naming

Names say what a thing is in its domain, never which language or pipeline stage
produced it. The OHDSI side keeps the definitions' names
(`definitions::TableRecord`, `definitions::FieldRecord`, the `cdmTableName` and
`cdmDatatype` cells); the generated side names the artefact (`lower::Table` is
one CDM table, `lower::Column` one column, `lower::RustType` the type a CDM
datatype maps to). A `Cdm`, `Omop` or `Gen` prefix on a type name is a smell: if
two views of one thing collide, the module path disambiguates
(`definitions::` versus `lower::`).
