<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Maintenance rule: every pull request that changes user-visible behaviour adds
an entry under **[Unreleased]** in the same PR. Cutting a release renames
[Unreleased] to the version and date, and adds a fresh link reference.

There is no release yet. FerroBRIDGE is in its design phase, and the
architecture is the output of the research program on
[issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1).

## [Unreleased]

### Changed

- `CONTRIBUTING.md` lists all six tier-1 gates `ci.yml` runs, in the same order
  and with the same flags. It named four, so a contributor who ran the listed
  commands could still fail CI on hadolint or the versions guard (#36).

### Added

- `.github/workflows/ci.yml`, the CI gate, in two tiers. Tier 1 runs on a
  repository with no code: zizmor at `--min-severity=low` with the online
  audits enabled, actionlint through its digest-pinned official image,
  shellcheck at `--severity=style` over every tracked shell program, hadolint
  over every tracked Dockerfile, and the comment-style and versions guards.
  Tier 2 is the Rust set (rustfmt, clippy, nextest and doctests, rustdoc,
  `cargo deny`, MSRV, dependency review), gated behind a `detect` job that
  looks for a root `Cargo.toml`, so it activates by itself when the workspace
  lands. The `conclusion` job reads every job's result and is the single
  required status check on `main`. Housekeeping the lanes read lands with it:
  `.hadolint.yaml`, `.dockerignore`, `.github/actionlint.yaml`, and the
  `.gitattributes` `linguist-generated` line reserved for the generated OMOP
  CDM subtree. `docs/ci-cd.md` records the design, why a mostly-skipped
  pipeline is still enforceable, and the owner actions that cannot be scripted
  (#16).
- The CI tool pins (zizmor, actionlint, shellcheck, hadolint) as a section of
  `docs/VERSIONS.md`, with `scripts/checks/versions.sh` failing when
  `ci.yml` and the matrix disagree (#16).

- `docs/VERSIONS.md`, the pin matrix: one place that records every version pin
  (the five specification pins, the published `fhir-types` and
  `fhir-terminology` crate pins, the Rust toolchain,
  edition, resolver and MSRV, the product version, and the licence) with the
  files that repeat each one. `scripts/checks/versions.sh` fails on any
  disagreement and on a first-party file claiming a licence other than
  `BUSL-1.1`, and skips with a printed reason for each file that does not
  exist yet. A `PostToolUse` hook runs the guard when a pinned file changes
  (#15).

### Changed

- The licence of the project's own code and text is the Business Source
  License 1.1 (`LICENSE`, `NOTICE`): free for non-production use and for non-commercial production use, a commercial
  licence for any other production use, and Apache License 2.0 four years
  after each version. Every header, the README badge and licensing section,
  and the community and governance documents name it (#12).

[Unreleased]: https://github.com/rubentalstra/FerroBRIDGE/commits/main
