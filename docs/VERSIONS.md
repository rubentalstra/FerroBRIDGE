<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
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
| OMOP CDM | v5.4 | `docs/architecture.md`, `tools/omop-cdm-codegen` and the `omop-cdm` crate constant, and its embedded DDL |
| openEHR ITS-REST | 1.1.0 | `docs/architecture.md`, the `ferrobridge-server` CDR module constant |

## Corpora and machine-readable inputs

A corpus is pinned by commit or immutable tag, never by a moving tag or a
`latest` URL, and vendored by a committed `scripts/vendor/*.sh` with a
`PROVENANCE.md` (`.claude/rules/vendored-inputs.md`). Each script below reads
its pin from this table, and `scripts/checks/versions.sh` reads each vendored
`PROVENANCE.md` back and fails when it names a different commit or tag.

| Item | Pin | Repeated in |
|---|---|---|
| FHIRconnect REST API chapter (draft, unmerged) | `SevKohler/FHIRconnect-spec` pull request #93 at head `2bf2a2fe91bae2ae659cda1665826567ea81b4af` | `docs/architecture.md` §2 and §4.7, `scripts/vendor/fhirconnect.sh`, `docs/specs/fhirconnect/draft-rest-api/PROVENANCE.md`, `crates/fhirconnect/tests/fixtures/draft-rest-api/PROVENANCE.md` |
| FHIRconnect specification source | `SevKohler/FHIRconnect-spec` commit `195b07fdb4c78da0432fdd1e9dbd127b81be6165` | `docs/architecture.md` §2, `scripts/vendor/fhirconnect.sh`, `docs/specs/fhirconnect/PROVENANCE.md` |
| FHIRconnect mapping library (corpus, never an oracle) | `SevKohler/FHIRconnect-mapping-lib` commit `6bd4c19a2f96821c04fbeed3c6f6c190fd85825b` | `docs/architecture.md` §2, `scripts/vendor/fhirconnect-mapping-lib.sh`, `docs/specs/fhirconnect-mapping-lib/PROVENANCE.md` |
| OMOCL corpus | `SevKohler/OMOCL` commit `c082db8ed81a062a574a2c058366045f600c2ed7` (grammar `OMOCL/v1.0.0`; the git tag `v1.0.0` carries pre-grammar files) | `docs/architecture.md` §2, `scripts/vendor/omocl.sh`, `docs/specs/omocl/PROVENANCE.md` |
| OMOP CDM definitions and PostgreSQL DDL | `OHDSI/CommonDataModel` tag `v5.4.3` | `docs/architecture.md` §2, `scripts/vendor/omop-cdm.sh`, `docs/specs/omop-cdm/PROVENANCE.md`, `tools/omop-cdm-codegen` and the banner of every file it emits |
| openEHR ITS-REST OpenAPI | `openEHR/specifications-ITS-REST` tag `Release-1.1.0`, modules EHR, Query, Definition, and the Simplified Formats and Simplified Data Template sources | `docs/architecture.md` §2, `scripts/vendor/its-rest.sh`, `docs/specs/its-rest/PROVENANCE.md` |
| KDS Diagnose operational template (fixture) | `openFHIR/openfhir` commit `5e4d68007518fcddae1907b333a86f495791aa53` path `core/src/test/resources/kds/diagnose/KDS_Diagnose.opt` sha256 `752483d90c4ba0f0d1e67baacceb67c1f6f698607823f2eebaa876f4f32bd870` | `scripts/vendor/kds-diagnose-opt.sh`, `tools/ferrobridge-testkit/fixtures/opt/kds/PROVENANCE.md` |
| HL7 v2 definitions (v2ig source of truth, never committed) | `HL7/v2ig` commit `3adcdbfff654ccbff5cd33aa34bd909a087e8e19` path `input/sourceOfTruth` files `1694` digest `9a1fb2b974b69f575bcd9b30fa66d2ff7f13ef7869c1350eca7462067e974f93` | `scripts/vendor/v2ig.sh`, `tools/fhir-codegen/vendor/hl7-v2ig/PROVENANCE.md`, the `codegen-drift` job of `.github/workflows/ci.yml` (its cache key) |
| HL7 v2 samples: Microsoft FHIR-Converter | `microsoft/FHIR-Converter` commit `70fd328e05019142f616a660cf65c6034baaa3c9` files `272` digest `8ba5602d3efc0f1161c1329887e3a9178983e1d2ed76f4353968aacd90c1420c` | `scripts/vendor/hl7v2-samples.sh`, `crates/ferrobridge-hl7v2/vendor/fhir-converter/PROVENANCE.md` |
| HL7 v2 samples: CDC ReportStream data tests | `CDCgov/prime-reportstream` commit `5f58b4c574fa64a70b362f7881f035ec6110a933` files `732` digest `857645f53e03efed22325254f1757773c3812f58f3b9b13f8cda796ba5415f3c` | `scripts/vendor/hl7v2-samples.sh`, `crates/ferrobridge-hl7v2/vendor/reportstream/PROVENANCE.md` |
| HL7 v2 samples: HL7 v2-to-FHIR benchmark messages | `HL7/v2-to-fhir` commit `873b331b3890c8bc5d62ef9b4dabb41801aac70d` files `15` digest `dd2831ec1bdef559421849bbe171213e8aa0761eb0105b362c969f4ebc23d8e6` | `scripts/vendor/hl7v2-samples.sh`, `crates/ferrobridge-hl7v2/vendor/v2-to-fhir/PROVENANCE.md` |
| HL7 v2 samples: NIST LRI (build time, never committed) | `usnistgov/hit-mu-tools-resource-bundles` commit `060f7af14daa359937f326c361bb9610b7a8ba48` (branch `lri-r2`) files `117` digest `666bafb90db5eeec7f727617fcbbe51b778065f93bc726f2746f524d70de7ec4` | `scripts/vendor/hl7v2-samples.sh --build-time`, `crates/ferrobridge-hl7v2/vendor/nist/PROVENANCE.md`, the `test` and `conformance` jobs of `.github/workflows/ci.yml` (their cache key) |
| HL7 v2 samples: NIST LOI (build time, never committed) | `usnistgov/hit-mu-tools-resource-bundles` commit `2c5e502787cd5e0fe0f347a7c00d5507a9f8035a` (branch `loi-r1`) files `65` digest `712d023d5a7ceb21ba520f513490fe1a29603436ed4756f48646d5922702f549` | `scripts/vendor/hl7v2-samples.sh --build-time`, `crates/ferrobridge-hl7v2/vendor/nist/PROVENANCE.md`, the same cache key |
| HL7 v2 samples: NIST syndromic surveillance (build time, never committed) | `usnistgov/hit-mu-tools-resource-bundles` commit `0d7a2c7b714188878b030068a94b94d1db6cb30a` (branch `ss-r2`) files `24` digest `aca07c08f2718139456f712ce8e45fc42cadba9c9fed665fd7ef5465c30af4c6` | `scripts/vendor/hl7v2-samples.sh --build-time`, `crates/ferrobridge-hl7v2/vendor/nist/PROVENANCE.md`, the same cache key |
| HL7 v2 samples: AIRA MQE (build time, never committed) | `immregistries/mqe` commit `8b8a4274e1830cdb78ff5c8627763e1625caeac7` path `examples` files `9` digest `1fbda5ede288bdaf9ea53b16ded2149f11627cf4b0b39d2837299ab5803972b7` | `scripts/vendor/hl7v2-samples.sh --build-time`, `crates/ferrobridge-hl7v2/vendor/aira-mqe/PROVENANCE.md`, the same cache key |
| HL7 v2+ licence page | `HL7/v2plus` commit `1a8fbb7e198047d71b9b13ceb582aaded4fa19f3` path `license.html` sha256 `48e2d1108ab38faafb9993342a9626b52d673ce2e683ef123716950564d76efd` | `scripts/vendor/v2ig.sh` (quoted verbatim in `tools/fhir-codegen/vendor/hl7-v2ig/PROVENANCE.md`) |

## openEHR model crates (crates.io)

The openEHR RM, the OPT 1.4 and Web Template codecs, the canonical JSON and
FLAT codecs, the ITS-REST data types and the AQL parser come from four
published crates, consumed by version like any other dependency
(`docs/architecture.md` §3). The minor line is pinned here; the manifest
carries the exact patch.

| Item | Pin | Repeated in |
|---|---|---|
| `openehr-base` | 0.0.72 | `docs/architecture.md`, later the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-rm` | 0.0.72 | `docs/architecture.md`, the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-its` | 0.0.72 | `docs/architecture.md`, the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-sdt` | 0.0.72 | `docs/architecture.md`, the root `Cargo.toml` `[workspace.dependencies]` (the Simplified Data Template engines, split out of `openehr-its` at 0.0.68) |
| `openehr-query` | 0.0.72 | `docs/architecture.md`, the root `Cargo.toml` `[workspace.dependencies]` |
| `openehr-am` | 0.0.72 | `docs/architecture.md`, the root `Cargo.toml` `[workspace.dependencies]` (the AOM2 OPT2 types an ADL 2 template decodes into) |
| `openehr-adl` | 0.0.72 | the root `Cargo.toml` `[workspace.dependencies]`, test-only: the ADL 2 test fixtures of `openehr-mapping-core` are compiled from their `.adls` sources |

## Third-party crates

A crate reaches this table only when `docs/architecture.md` §2 grounds the
choice by name, so the ground and the pin never drift apart. Everything else
stays in the root `Cargo.toml` `[workspace.dependencies]` alone, per §Rust
dependency pins below.

| Item | Pin | Repeated in |
|---|---|---|
| `serde-saphyr` | 1.3.0 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` |
| `jsonschema` | 0.57.0 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the FHIRconnect mapping schemas) |
| `reqwest` | 0.13.5 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` |
| `backon` | 1.6.0 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` |
| `url` | 2.5.8 | the root `Cargo.toml` `[workspace.dependencies]` |
| `http` | 1.5.0 | the root `Cargo.toml` `[workspace.dependencies]` |
| `secrecy` | 0.10.3 | the root `Cargo.toml` `[workspace.dependencies]` |
| `tracing` | 0.1.44 | the root `Cargo.toml` `[workspace.dependencies]` |
| `tracing-subscriber` | 0.3.23 | the root `Cargo.toml` `[workspace.dependencies]` (the server console) |
| `axum` | 0.8.9 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` |
| `tower-http` | 0.7.1 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` |
| `tower` | 0.5.3 | the root `Cargo.toml` `[workspace.dependencies]` |
| `toml` | 1.1.6 | the root `Cargo.toml` `[workspace.dependencies]` (the server configuration file) |
| `uuid` | 1.26.1 | the root `Cargo.toml` `[workspace.dependencies]` (the minted request id) |
| `redb` | 4.3.0 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the facade identity store) |
| `jiff` | 0.2.37 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the bridge's own timestamps) |
| `futures-core`, `futures-util` | 0.3.34 | the root `Cargo.toml` `[workspace.dependencies]` |
| `tokio` | 1.53.1 | the root `Cargo.toml` `[workspace.dependencies]` |
| `wiremock` | 0.6.5 | the root `Cargo.toml` `[workspace.dependencies]` (tests only) |
| `testcontainers` | 0.28.0 | the root `Cargo.toml` `[workspace.dependencies]` (tests only) |
| `tokio-postgres` | 0.7.18 | the root `Cargo.toml` `[workspace.dependencies]` (the CDM writer's client, and the tests that read the CDM back) |
| `tokio-postgres-rustls` | 0.14.0 | the root `Cargo.toml` `[workspace.dependencies]` (the CDM writer's TLS connector, on the aws-lc-rs provider) |
| `rustls` | 0.23.45 | the root `Cargo.toml` `[workspace.dependencies]` (the CDM writer's certificate checks after `sslmode`) |
| `webpki-roots` | 1.0.9 | the root `Cargo.toml` `[workspace.dependencies]` (the root set both CDM clients trust when no CA is configured) |
| `sqlx` | 0.9.0 | the root `Cargo.toml` `[workspace.dependencies]` (the concept resolver; the `sqlx-cli` row carries the same version) |
| `rust_decimal` | 1.43.0 | the root `Cargo.toml` `[workspace.dependencies]` (the exact decimal of the OMOCL engine; MIT) |
| `tokio-util` | 0.7.19 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the MLLP frame codec of the HL7 v2 face) |
| `bytes` | 1.12.1 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the MLLP codec's buffer) |
| `encoding_rs` | 0.8.42 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the MSH-18 character sets) |
| `logos` | 0.16.1 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the v2-to-FHIR condition and target lexers) |
| `chumsky` | 0.13.0 | `docs/architecture.md` §7, the root `Cargo.toml` `[workspace.dependencies]` (the v2-to-FHIR condition and target parsers) |

## Container images (the end-to-end lane)

The end-to-end lane starts real servers in containers behind the
`FERROBRIDGE_E2E` gate (`docs/ci-cd.md`). Every image is pinned by tag AND by
the digest of its image index, because a tag can be re-pointed and a digest
cannot; the digest is what Docker resolves. The one place each pin is written
in code is the matching constant in
`tools/ferrobridge-testkit/src/containers.rs`, and
`scripts/checks/versions.sh` compares the two.

| Item | Pin | Repeated in |
|---|---|---|
| PostgreSQL image | `postgres:18.6@sha256:4ef4dbc939d61acea57712655ddb4b4ab27419c913f94cca0cd57cb3ea3c2280` | the `POSTGRES` constant in `tools/ferrobridge-testkit/src/containers.rs`, the `sqlx-offline` service in `.github/workflows/ci.yml` |
| FerroEHR CDR image | `ghcr.io/rubentalstra/ferroehr:4.2.5@sha256:aa5a9e0447befadb396084fd19ce7a6d30ea1fb8e16806615071e9f1748125ca` | the `CDR` constant in `tools/ferrobridge-testkit/src/containers.rs` |
| FerroEHR CDR database image | `ghcr.io/rubentalstra/ferroehr-postgres:4.2.5@sha256:e094461744fa8510ca8c1c4ecde4460474befb00b310ba41f7d9ff6e67e181bc` | the `CDR_POSTGRES` constant in `tools/ferrobridge-testkit/src/containers.rs` |
| FerroTERM terminology server image | `ghcr.io/rubentalstra/ferroterm:0.1.3@sha256:b1ef80382e03c2474bfec2ec57a698d83314e1290dd0cb5a2612ea208bde020c` | the `TERMINOLOGY` constant in `tools/ferrobridge-testkit/src/containers.rs` |

The CDR runs against its own published database image rather than the plain
PostgreSQL above: that image is built from `postgres:18.6` and carries the
login role, the schemas and the extensions the CDR's migrations expect to find,
which a bare PostgreSQL does not have.

### The shipped image and the quickstart

`docker/Dockerfile` builds on one more image, pinned by the digest of its image
index, which is what Docker resolves for a multi-platform build.
`scripts/checks/versions.sh` reads its `FROM` line back against this row, and
Dependabot (`docker`, over `/docker`) proposes the bumps.

| Item | Pin | Repeated in |
|---|---|---|
| Container base image | `gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3` | the `FROM` of `docker/Dockerfile`, and its `org.opencontainers.image.base.name` label without the digest |

The quickstart `compose.yaml` pulls `ghcr.io/rubentalstra/ferrobridge` at the
release it shipped with, so its tag is the product version below rather than a
pin of its own, and the guard compares the two. Its CDM service runs the same
`postgres:18.6` image the row above pins for the end-to-end lane, so a
PostgreSQL bump moves one row and both readers follow it.

The terminology server needs no database. It reads the synthetic `CodeSystem`,
`ValueSet` and `ConceptMap` under
`tools/ferrobridge-testkit/fixtures/terminology` from a read-only bind mount
named by `FERROTERM_CODESYSTEMS`, and serves each FHIR release under its own
path prefix (`/r4`, `/r4b`).

## FHIR packages (the `fhir-types` generator input)

The FHIR model is generated by `tools/fhir-codegen` from the HL7 FHIR packages
below, vendored verbatim from the FHIR package registry by
`scripts/vendor/fhir-packages.sh`, which reads its pins from this table. Owner
decision 2026-09-05: the crate and its generator move here from the sibling
terminology server, which then consumes the crate from crates.io. The first
increment of #72 landed both trees, so the `PROVENANCE.md` paths below exist
and the `codegen-drift` CI job reads the packages on every run.
`hl7.fhir.uv.v2mappings` is no input of `fhir-types`: it carries the
v2-to-FHIR ConceptMaps for the HL7 v2 face (#251), and the script takes the
`LICENSE` of its source repository beside it because that licence differs
from the package's own. `scripts/checks/versions.sh` reads the `Version` line
of every `PROVENANCE.md` below back against its row.

| Item | Pin | Repeated in |
|---|---|---|
| `hl7.fhir.r4.core` | 4.0.1 | `tools/fhir-codegen/vendor/hl7.fhir.r4.core/PROVENANCE.md` |
| `hl7.fhir.r4b.core` | 4.3.0 | `tools/fhir-codegen/vendor/hl7.fhir.r4b.core/PROVENANCE.md` |
| `hl7.fhir.r5.core` | 5.0.0 | `tools/fhir-codegen/vendor/hl7.fhir.r5.core/PROVENANCE.md` |
| `hl7.fhir.r6.core` | 6.0.0-ballot5 | `tools/fhir-codegen/vendor/hl7.fhir.r6.core/PROVENANCE.md` |
| `hl7.terminology` (THO) | 7.3.0 | `tools/fhir-codegen/vendor/hl7.terminology/PROVENANCE.md` |
| `hl7.fhir.uv.v2mappings` (the v2-to-FHIR IG) | 1.0.0 | `tools/fhir-codegen/vendor/hl7.fhir.uv.v2mappings/PROVENANCE.md` (FHIR 4.0.1; the package declares CC0-1.0, its source repository `HL7/v2-to-fhir` Apache-2.0) |

## Profile packages (the mapping targets)

No profile package is pinned yet. A row is added here the moment a context
mapping targeting the package is authored (`docs/architecture.md` §4.8 names
the candidates and their order: `hl7.fhir.eu.base`, `hl7.fhir.eu.laboratory`,
`hl7.fhir.eu.mpd`, then `hl7.fhir.eu.eps`; the mapping library's own contexts
target the MII Kerndatensatz modules at 2025.0.0).

## Crate line

The library crates are published to crates.io on one crate version line,
distinct from the product version (owner decision 2026-09-05). The line is
`fhir-types`' own: the sibling published 0.1.97 on 2026-09-12, the first
release from here was 0.1.98, and the line moves one patch with every change
to the crate's packaged content (`.claude/rules/crates-publishing.md`). Every
other library crate holds its name with a 0.0.0 placeholder (#107), outside
the line until its first publish, when it joins at the line's current value.

| Item | Pin | Repeated in |
|---|---|---|
| `fhir-types` | 0.1.106 | `docs/architecture.md`, `crates/fhir-types/Cargo.toml`, the root `Cargo.toml` `[workspace.dependencies]`, later the `version` of every published `crates/*` manifest |

## Language and runtime

`rust-toolchain.toml` carries the toolchain, and the root `Cargo.toml` carries
the edition, the resolver and the MSRV (#20). The release lane builds every
published binary on this toolchain, with no cache.

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
version is 0.0.3.

| Item | Pin | Repeated in |
|---|---|---|
| Product version | 0.0.3 | root `Cargo.toml` `[workspace.package]` `version` (#20), `CITATION.cff` `version` (#18), the `ghcr.io/rubentalstra/ferrobridge` image tag default in `compose.yaml` (#22) |

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

`LICENSE` names Apache License 2.0 as a licence of its own, as the Change
License four years after each version. `crates/fhir-types` is the one
first-party crate published under Apache-2.0 (`docs/architecture.md` §4.1): it
is generated from the CC0 HL7 FHIR packages and three servers depend on it, so
`scripts/checks/versions.sh` excludes that path and fails on an Apache-2.0 or
MIT claim anywhere else.

Third-party and vendored material keeps its upstream terms, recorded beside the
vendored tree (`.claude/rules/vendored-inputs.md`).

## Rust dependency pins

The root `Cargo.toml` `[workspace.dependencies]` table is the authoritative,
fully pinned third-party crate set. Beyond the openEHR model crates and the
§Third-party crates table above, this file does not duplicate crate versions;
on any discrepancy the manifest wins. A crate joins a member with
`dep.workspace = true`.

## CI tool pins

The tier-1 lanes of `.github/workflows/ci.yml` run four analyzers, each pinned
to an exact version so a CI result matches the local one. `zizmor` and
`shellcheck` are fetched by `taiki-e/install-action`, which verifies the
upstream release checksum; `actionlint` and `hadolint` run from their official
container images, pinned by tag and by digest. The `sqlx-offline` job installs
`sqlx-cli` through the same action, which falls back to `cargo-binstall` because
`sqlx-cli` is not in its own tool list. `sqlx-cli` carries the
`sqlx` crate's version, because the CLI writes the query metadata the crate's
macros read.

| Item | Pin | Repeated in |
|---|---|---|
| `zizmor` | 1.30.1 | `.github/workflows/ci.yml` |
| `actionlint` | 1.7.12 | `.github/workflows/ci.yml` |
| `shellcheck` | 0.11.0 | `.github/workflows/ci.yml` |
| `hadolint` | 2.15.1 | `.github/workflows/ci.yml` |
| `sqlx-cli` | 0.9.0 | `.github/workflows/ci.yml` (the `sqlx-offline` job) |

Keep the locally installed versions on these numbers, so a finding costs a
local run rather than a CI round trip (`.claude/rules/ci-cd.md`).

## Release tool pins

The release lane builds and describes every published artifact with three more
tools, each fetched by the same digest-pinned `taiki-e/install-action`, which
verifies the upstream release checksum. They decide what a consumer can prove
about a binary, so a floating version here would change the contents of a
release without a reviewed change (`docs/release.md`).

| Item | Pin | Repeated in |
|---|---|---|
| `cargo-auditable` | 0.7.5 | `.github/workflows/release-build.yml` |
| `cargo-cyclonedx` | 0.5.9 | `.github/workflows/release-build.yml` |
| `cargo-fuzz` | 0.13.2 | `.github/workflows/fuzz.yml` (the fuzz lane, the one nightly-toolchain job; #155) |
| `syft` | 1.51.1 | `.github/workflows/release-build.yml`, `.github/workflows/release-image.yml` |

`scripts/checks/versions.sh` reads every `tool:` line of the two release
workflows back against these rows, so a bump moves one row and the workflows
follow it.

## GitHub Actions pins

Every `uses:` in `.github/workflows/**` is pinned to a full commit SHA with a
trailing `# vX.Y.Z` comment (`.claude/rules/ci-cd.md`). Dependabot bumps them,
and zizmor checks the form (#16).
