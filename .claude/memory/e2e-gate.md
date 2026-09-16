---
name: e2e-gate
description: Container-backed tests run only with FERROBRIDGE_E2E=1 through the testkit harness (PostgreSQL 18.6, the reference CDR, the reference terminology server, all pinned by digest); unset, they skip and the suite is offline
metadata:
  type: project
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

Landed with #78 (2026-09-12). `tools/ferrobridge-testkit/src/containers.rs` is
the harness: `postgres()`, `cdr()` and, with #77, `terminology()` start the
pinned images through testcontainers and hand back connection URLs; every
container test begins with the gate check and returns early when
`FERROBRIDGE_E2E` is not `1`, so `cargo nextest run --workspace` stays offline
and green. CI runs the gated tests in the `e2e (containers)` job, per package.

**How to apply:** a new container-backed test uses the harness and the gate,
never its own `docker` calls; a new image is a `PinnedImage` constant plus a
`docs/VERSIONS.md` row, which `scripts/checks/versions.sh` compares. Locally:
`FERROBRIDGE_E2E=1 cargo nextest run -p <crate>` with Docker running.
