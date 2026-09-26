---
name: ci-runner-memory
description: A workspace-wide cargo check, clippy or instrumented test build is killed on the hosted runner (exit 143, no diagnostic) because two generated giants compile side by side; lint and check one package at a time with cargo hack, and never lint the all-features union of fhir-types in CI
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-12 the `clippy` job died four times with "The runner has received
a shutdown signal" and exit 143 while every fix that only capped parallelism
failed; the `msrv` job compiling the same crates through `cargo hack check`
passed every time.

**Why:** the hosted runner has seven gigabytes. A workspace-wide `cargo check`
or `cargo clippy` schedules `fhir-types` (four versions) and `openehr-am` (the
AM 2.4 model behind the ITS-REST client) at the same time; per package they
never meet. The all-features union of `fhir-types` (four versions, every
resource, one crate) is larger still and no consumer builds it.

**How to apply:** CI lints with `cargo hack clippy --workspace --all-targets
--locked -- -D warnings` (per package) and lints `fhir-types` per version with
`resources` in a matrix; `CARGO_BUILD_JOBS=2` stays at workflow level. A new
job that compiles the workspace uses the per-package form from the start.
Locally the all-features clippy still runs on a machine with the memory. A
143 with no diagnostic is memory, never a lint: read the log for the runner
shutdown line before changing code.

The same shape killed the SonarQube coverage lane on 2026-09-13 (#146): its
`cargo llvm-cov nextest --workspace --all-features` step never produced an
lcov file, so the coverage Sonar imported was nothing. The lane now runs
`cargo llvm-cov nextest --no-report -p <package>` per workspace member and
merges with `cargo llvm-cov report`; the step fails when the report has zero
line records, so a silent empty import cannot come back.

The `test` job met the same kill on 2026-09-25 (#235) once `omocl` took
`omop-cdm` with `sqlx`: `cargo nextest run --workspace` died at exit 143 after
five minutes with no diagnostic. It now runs `cargo hack nextest run
--workspace` and `cargo hack test --doc --workspace`, one package at a time,
like the clippy and msrv lanes.


**Full-emit tests (2026-09-26):** after #303 grew `hl7v2-types` to 2038 files,
the three v2 full-emit tests took 368 s to 819 s each under coverage and the
SonarQube Cloud and `test` jobs hit their 45-minute timeouts, queueing
eighteen runs. Any test that emits a whole generated crate goes behind
`FERROBRIDGE_CODEGEN_EMIT=1` and runs only in `codegen-drift`, in release.
