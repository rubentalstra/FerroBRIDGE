<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Pinned version matrix

This file is the single source of truth for every version pin in FerroBRIDGE.
When it and a file that repeats a pin disagree, that is drift. Fix the
disagreement; never let either side silently win.
`scripts/checks/versions.sh` enforces the cross-file agreement it can reach and
skips loudly for the files that do not exist yet, so the guard is useful on a
tree with no Cargo workspace and grows teeth as files appear.

No specification governs this file; it is FerroBRIDGE's own design.

## Specifications

The ground for each pin is the pin table in `docs/architecture.md` §2, which
records why the value is what it is. The guard compares the first token of each
`Pin` cell below with the first token of the same row there.

| Item | Pin | Repeated in |
|---|---|---|
| FHIRconnect | v1.0.0 | `docs/architecture.md`, later the `fhirconnect-*` crates |
| FHIR | R4 (4.0.1) | `docs/architecture.md`, later the facade and terminology crates |
| OMOCL | v1.0.0 | `docs/architecture.md`, later the `omocl-*` crates |
| OMOP CDM | v5.4 | `docs/architecture.md`, later the `omop-cdm` generator and its DDL |
| openEHR ITS-REST | 1.1.0 | `docs/architecture.md`, later the `ferrobridge-openehr` client |

## FHIR model crates (crates.io)

The FHIR model and the terminology operation contracts come from two published
crates, consumed by version like any other dependency (`docs/architecture.md`
§7).

| Item | Pin | Repeated in |
|---|---|---|
| `fhir-types` | 0.1.22 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |
| `fhir-terminology` | 0.1.22 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |

## Language and runtime

No Cargo workspace exists yet. Issue #20 stands one up and adopts every value
below: `rust-toolchain.toml` carries the toolchain, and the root `Cargo.toml`
carries the edition, the resolver, and the MSRV.

| Item | Pin | Repeated in |
|---|---|---|
| Rust toolchain | 1.98.1 | `rust-toolchain.toml` `channel` (stable) |
| Edition | 2024 | root `Cargo.toml` `[workspace.package]` `edition` |
| Cargo resolver | 3 | root `Cargo.toml` `[workspace]` `resolver` |
| MSRV | 1.98 | root `Cargo.toml` `[workspace.package]` `rust-version` |

The deliverable is a server binary, so the MSRV tracks the pinned stable
toolchain.

## Product and citation version

The product version is the workspace `version` in the root `Cargo.toml`, which
every member inherits. The milestone line is 0.0.x, so the first product
version is 0.0.1.

| Item | Pin | Repeated in |
|---|---|---|
| Product version | 0.0.1 | root `Cargo.toml` `[workspace.package]` `version` (#20), `CITATION.cff` `version` (#18) |

`CITATION.cff` tracks the workspace version exactly. The guard compares the two
once both files exist.

## Licence

| Item | Pin | Repeated in |
|---|---|---|
| Project licence | BUSL-1.1 | `LICENSE`, `NOTICE`, the SPDX header of every first-party file, later the `license` field of every own `Cargo.toml`, the container `image.licenses` label, the README badge |

`LICENSE` is the one file that names Apache License 2.0 as a licence of its
own, as the Change License four years after each version. The guard fails when
any other first-party file claims MIT or Apache-2.0 as its own licence.

Third-party and vendored material keeps its upstream terms, recorded beside the
vendored tree (`.claude/rules/vendored-inputs.md`). The crate table in
`docs/architecture.md` records `fhir-types` as Apache-2.0 because that is the
licence of that crate on crates.io, which says nothing about the licence of
FerroBRIDGE's own code.

## Rust dependency pins

The root `Cargo.toml` `[workspace.dependencies]` table is the authoritative,
fully pinned third-party crate set once it exists. This file does not duplicate
crate versions; on any discrepancy the manifest wins. A crate joins a member
with `dep.workspace = true`.

## CI tool pins

The tier-1 lanes of `.github/workflows/ci.yml` run four analyzers, each pinned
to an exact version so a CI result matches the local one. `zizmor` and
`shellcheck` are fetched by `taiki-e/install-action`, which verifies the
upstream release checksum; `actionlint` and `hadolint` run from their official
container images, pinned by tag and by digest.

| Item | Pin | Repeated in |
|---|---|---|
| `zizmor` | 1.29.0 | `.github/workflows/ci.yml` |
| `actionlint` | 1.7.12 | `.github/workflows/ci.yml` |
| `shellcheck` | 0.11.0 | `.github/workflows/ci.yml` |
| `hadolint` | 2.15.1 | `.github/workflows/ci.yml` |

Keep the locally installed versions on these numbers, so a finding costs a
local run rather than a CI round trip (`.claude/rules/ci-cd.md`).

## GitHub Actions pins

Every `uses:` in `.github/workflows/**` is pinned to a full commit SHA with a
trailing `# vX.Y.Z` comment (`.claude/rules/ci-cd.md`). Dependabot bumps them,
and zizmor checks the form (#16).
