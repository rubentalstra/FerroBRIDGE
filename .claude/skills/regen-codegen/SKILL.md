---
name: regen-codegen
description: Regenerate the generated layers, fhir-types from the vendored FHIR packages and omop-cdm from the vendored OHDSI definitions, and verify no drift. Use after changing a generator, an override, or a vendored pin.
allowed-tools: Bash, Read, Grep
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Regenerate the FHIR layer

`crates/fhir-types` is generated from the vendored, pinned FHIR packages by
`tools/fhir-codegen`. Never hand-edit a `// @generated` file: change the
generator or its override map and regenerate here. Full discipline:
`.claude/rules/codegen.md`.

## Steps

1. **Confirm the inputs are the pinned packages.** The vendored FHIR packages
   (`hl7.fhir.r4.core`, `hl7.fhir.r4b.core`, `hl7.fhir.r5.core`,
   `hl7.fhir.r6.core`, `hl7.terminology`) live under
   `tools/fhir-codegen/vendor/` with a `PROVENANCE.md` each. They are fetched
   only by `scripts/vendor/fhir-packages.sh`, which reads its pins from
   `docs/VERSIONS.md`, and are never hand-edited
   (`.claude/rules/vendored-inputs.md`).

2. **Regenerate** the per-version modules:

   ```bash
   cargo run -p fhir-codegen -- emit
   ```

3. **Verify no drift:** regeneration is byte-deterministic and the working tree
   is clean afterward:

   ```bash
   git diff --exit-code crates/fhir-types
   ```

   A non-empty diff means the committed generated code was stale; commit the
   regenerated output. The `codegen-drift` job in `.github/workflows/ci.yml`
   runs `cargo run --locked -p fhir-codegen -- emit --check` as the gate, over
   the union tree: one emitted tree holds every declared root set and each item
   carries the `cfg` of the narrowest feature that selects it, so the drift
   check covers every feature at once.

4. **Gate the result:**

   ```bash
   cargo build -p fhir-types
   cargo clippy -p fhir-types --all-targets -- -D warnings
   cargo hack check -p fhir-types --each-feature --locked
   cargo nextest run -p fhir-codegen
   ```

## Regenerate the OMOP layer

`crates/omop-cdm/src/generated/` is generated from
`docs/specs/omop-cdm/inst/csv/` by `tools/omop-cdm-codegen`, which also copies
the four rendered PostgreSQL DDL files into `crates/omop-cdm/ddl/`. Both trees
are off-limits to hand edits.

```bash
cargo run -p omop-cdm-codegen -- emit
git diff --exit-code crates/omop-cdm
cargo clippy -p omop-cdm --all-targets -- -D warnings
cargo nextest run -p omop-cdm -p omop-cdm-codegen
```

`cargo run --locked -p omop-cdm-codegen -- emit --check` is the gate, run by
the `omop-cdm-codegen-drift` job in `.github/workflows/ci.yml`. The inputs are
fetched only by `scripts/vendor/omop-cdm.sh` from the `docs/VERSIONS.md` pin.

## Rules

- The generator emits the COMPLETE model from the vendored inputs; never trim
  output to quiet a diff or dodge a build error.
- A shape a consumer needs but the generated crate lacks is fixed in the
  generator or its override, never with a hand-written shadow type.
- Every generated file starts with `// @generated` and is off-limits to hand
  edits.
