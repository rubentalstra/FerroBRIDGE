---
name: crates-published
description: "Owner decision 2026-09-05: FerroBRIDGE publishes its library crates to crates.io (the mapping foundation, the FHIRconnect and OMOCL crates, the OMOP CDM crate), so the release lane carries a crates.io leg and the crate public surfaces are designed as published API from the first increment"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On **2026-09-05** the owner decided that FerroBRIDGE **publishes its library
crates to crates.io**, on the same model as the sibling terminology server
(`crates/*` published, `app/*` and `tools/*` never). The server binary and the
container image remain the product; the crates are a second deliverable.

**Why:** the owner was asked "unpublished, publish, or design for it later"
and chose publish outright. The crates carry the mapping foundation other
Rust projects can reuse, and the sibling's lane is proven.

**How to apply:**

- The release lane (#23) gains a crates.io leg: a publish workflow in
  dependency order, a crate-version guard, a dry-run gate, and crates.io
  Trusted Publishing where the sibling uses it. Read the sibling's
  `.claude/rules/crates-publishing.md` and lane before designing ours; port
  the rule into this repository's `.claude/rules/`.
- `crates/*` members set `publish = true` with `include` lists; `app/*` and
  `tools/*` stay `publish = false`. Every published crate carries `README.md`,
  `LICENSE`, `documentation`, `keywords`, `categories`.
- A published crate's `pub` surface is API: zero re-exports, deliberate
  visibility, `#[non_exhaustive]` where a spec enum may grow, semver
  discipline from 0.1.0 on. C-STABLE in `reliability.md` §Deviations is
  re-read the moment the first crate ships: pre-1.0 public dependencies in a
  published API are re-adjudicated then.
- Published crates carry a lockstep crate version line distinct from the
  product version, as the sibling does (`fhir-types` 0.1.43 beside product
  0.0.11 on 2026-09-05).
- The crates depend on the openEHR model crates and the FHIR model crates by
  version from crates.io, never by path, so a consumer can build them.
- **The FHIR model crate moves to FerroBRIDGE.** Owner ruling 2026-09-05, after
  two rounds: `fhir-types` and its generator `fhir-codegen` (with the vendored
  HL7 packages and the drift gate) move from FerroTERM into this repository,
  which generates the full FHIR model (every resource, per-version features,
  the public element table). FerroTERM then consumes `fhir-types` from
  crates.io behind a terminology-only feature set. FerroTERM #298 (the widening
  in place) was closed as not planned and FerroTERM #300 (v0.1.1) carries the
  sibling's half of the move; a separate repository for the crate was
  refused ("we will not have an own thing"). The bridge is the crate's
  largest consumer, so the generator lives where the root set is widest.