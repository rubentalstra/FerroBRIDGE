# hl7v2-types

The generated HL7 v2 layer. Every `.rs` file here is produced by
`tools/fhir-codegen` (its `v2` root set) from the HL7 v2 definitions fetched
into `tools/fhir-codegen/vendor/hl7-v2ig/` by `scripts/vendor/v2ig.sh`, and is
off-limits to hand edits; the only hand-maintained files are `Cargo.toml`,
`README.md`, `LICENSE` and this `CLAUDE.md`. Full discipline:
`.claude/rules/codegen.md`.

- To change anything under `src/`, change the emitter (`tools/fhir-codegen/src/v2/`
  and `src/templates/hl7v2_model.rs`), then run
  `cargo run -p fhir-codegen -- emit` and commit the regenerated tree.
  `cargo run -p fhir-codegen -- emit --check` is the CI drift check and fails
  on any difference. Both need the definitions on disk: run
  `scripts/vendor/v2ig.sh` first.
- The root set is every message structure under
  `message-structure/message_structures/` and every segment definition under
  `segment/segments/` (the `Hxx` slot file aside), so the batch envelopes and
  the segments no structure references are emitted too. Data type
  components are not emitted (the v2-to-FHIR `TypeInfo` extensions type the
  fields for the mapper), and neither are table contents (`hl7.terminology`
  carries them).
  Widening the root set is a recorded decision, never a per-file addition.
- The emitter tolerates each defect of the definitions only in the files
  where it was found (`fhir_codegen::v2::lower::Defect`); the same defect
  anywhere else fails the emit.
- The crate is Apache-2.0, the second first-party crate under a licence other
  than BUSL-1.1 beside `fhir-types` (owner ruling on #251: generated Rust is
  the project's own code); `scripts/checks/versions.sh` carries the
  exception. The fetched definitions are never packaged or committed.
- `doctest = false` is deliberate: generated doc text is not a curated
  example set.
