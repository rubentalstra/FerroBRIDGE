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

## Corpora and machine-readable inputs

A corpus is pinned by commit or immutable tag, never by a moving tag or a
`latest` URL, and vendored by a committed `scripts/vendor/*.sh` with a
`PROVENANCE.md` (`.claude/rules/vendored-inputs.md`). The guard does not read
these rows yet; the vendor scripts do, when they land (#20).

| Item | Pin | Repeated in |
|---|---|---|
| FHIRconnect specification source | `SevKohler/FHIRconnect-spec` commit `195b07fdb4c78da0432fdd1e9dbd127b81be6165` | `docs/architecture.md` §2, later `scripts/vendor/fhirconnect.sh` |
| FHIRconnect mapping library (corpus, never an oracle) | `SevKohler/FHIRconnect-mapping-lib` commit `6bd4c19a2f96821c04fbeed3c6f6c190fd85825b` | `docs/architecture.md` §2, later `scripts/vendor/fhirconnect.sh` |
| OMOCL corpus | `SevKohler/OMOCL` commit `dd42574fdb074c02cbe077a0c49b1bb5bae28f35` (grammar `OMOCL/v1.0.0`; the git tag `v1.0.0` carries pre-grammar files) | `docs/architecture.md` §2, later `scripts/vendor/omocl.sh` |
| OMOP CDM definitions and PostgreSQL DDL | `OHDSI/CommonDataModel` tag `v5.4.3` | `docs/architecture.md` §2, later `scripts/vendor/omop-cdm.sh` and the `omop-cdm` generator |
| openEHR ITS-REST OpenAPI | `openEHR/specifications-ITS-REST` tag `Release-1.1.0`, modules EHR, Query, Definition | `docs/architecture.md` §2, later `scripts/vendor/its-rest.sh` |

## openEHR model crates (crates.io)

The openEHR RM, the OPT 1.4 and Web Template codecs, the canonical JSON and
FLAT codecs, the ITS-REST data types and the AQL parser come from four
published crates, consumed by version like any other dependency
(`docs/architecture.md` §3). The minor line is pinned here; the manifest
carries the exact patch.

| Item | Pin | Repeated in |
|---|---|---|
| `openehr-base` | 0.0.61 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-rm` | 0.0.61 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-its` | 0.0.61 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-query` | 0.0.61 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |

## FHIR packages (the `fhir-types` generator input)

The FHIR model is generated in this repository by `tools/fhir-codegen` from the
HL7 FHIR packages below, vendored verbatim from the FHIR package registry
(owner decision 2026-09-05: the crate and its generator move here from the
sibling terminology server, which then consumes the crate from crates.io).

| Item | Pin | Repeated in |
|---|---|---|
| `hl7.fhir.r4.core` | 4.0.1 | `tools/fhir-codegen/vendor/hl7.fhir.r4.core/PROVENANCE.md` |
| `hl7.fhir.r4b.core` | 4.3.0 | `tools/fhir-codegen/vendor/hl7.fhir.r4b.core/PROVENANCE.md` |
| `hl7.fhir.r5.core` | 5.0.0 | `tools/fhir-codegen/vendor/hl7.fhir.r5.core/PROVENANCE.md` |
| `hl7.fhir.r6.core` | 6.0.0-ballot5 | `tools/fhir-codegen/vendor/hl7.fhir.r6.core/PROVENANCE.md` |
| `hl7.terminology` (THO) | 7.3.0 | `tools/fhir-codegen/vendor/hl7.terminology/PROVENANCE.md` |

## Crate line

The library crates are published to crates.io on one lockstep crate version
line, distinct from the product version (owner decision 2026-09-05). The line
inherits `fhir-types`, whose last release from the sibling is the floor. Every
other library crate holds its name with a 0.0.0 placeholder (#107), a version
outside the line.

| Item | Pin | Repeated in |
|---|---|---|
| `fhir-types` | 0.1.43 | `docs/architecture.md`, later the `version` of every published `crates/*` manifest |

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

`CITATION.cff` tracks this row exactly, and the guard compares the two whenever
`CITATION.cff` exists. Once the root `Cargo.toml` lands, the guard also compares
its `[workspace.package]` `version` with both.

## Documentation toolchain

The site is an mdBook rendered by `.github/workflows/docs.yml`. Every tool it
installs is pinned here and repeated in the composite action that installs
them, so the book renders the same way in CI as it does on a laptop.

| Item | Pin | Repeated in |
|---|---|---|
| mdBook | 0.5.4 | `.github/actions/docs-toolchain/action.yml` `mdbook-version` |
| mdbook-toc | 0.15.4 | `.github/actions/docs-toolchain/action.yml` `mdbook-toc-version` |
| mdbook-mermaid | 0.17.1 | `.github/actions/docs-toolchain/action.yml` `mdbook-mermaid-version` |

The mermaid browser assets the book loads are vendored from the same
`mdbook-mermaid` release, pinned by commit in
`scripts/vendor/mdbook-mermaid-assets.sh` and recorded in
`website/book/vendor/mermaid/PROVENANCE.md`.

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
