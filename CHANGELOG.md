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

### Added

- `.github/release.yml`, so GitHub's auto-generated release notes group pull
  requests by the tracker's own label taxonomy (Features, Fixes, Security,
  Documentation, Dependencies, Maintenance, Other changes), with
  `no-changelog` as the exclusion. The hand-curated changelog stays the primary
  record (#17).
- Two `cargo` and `docker` entries in `.github/dependabot.yml` beside the
  existing `github-actions` one, each grouping minor and patch bumps into one
  weekly pull request while majors stay individual, with the conventional
  commit prefixes `ci`, `deps` and `build` and `open-pull-requests-limit: 10`
  on cargo. Both are inert until their manifests exist (#17).
- The Rust half of `.github/workflows/sonar.yml`, gated on a root `Cargo.toml`
  at step level: the pinned toolchain with `llvm-tools-preview`, the
  instrumented `cargo llvm-cov nextest` run that writes `lcov.info`, and the
  `sonar.projectVersion` derivation that anchors the New Code window to the
  workspace version. `sonar-project.properties` names the lcov file and
  excludes build output, vendored trees, and the generated OMOP CDM subtree
  (#17).
- The documentation site at <https://ferrobridge.eu/>: a hand-written landing
  page at the root and an mdBook under `/docs/`, organised by reader intent
  (Evaluate, Operate, Integrate, Contribute). `.github/workflows/docs.yml`
  builds it on every pull request and deploys from `main` through GitHub Pages,
  `.github/actions/docs-toolchain/action.yml` installs the pinned mdBook
  toolchain, and `scripts/site/assemble.sh` places both halves and renders the
  roadmap block from the open milestones. `llms.txt` and `CITATION.cff` land
  with it, the README badge block covers CI, CodeQL, Scorecard, the Sonar
  quality gate and coverage, the licence and the latest release, and
  `scripts/checks/versions.sh` now guards the citation version and the three
  documentation-toolchain pins (#18).
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

- `SUPPORT.md` sends a reader to the documentation site first. It still said
  there was no site, which went false the day the site went live and
  contradicted `README.md` on the same tree (#44).

- The contributor page at `/docs/contribute/checks-and-gates.html` names the
  zizmor audit path `.github/`, matching what CI runs. It still named
  `.github/workflows/`, so following it missed the Dependabot configuration
  and everything under `.github/actions/` (#40).

- The zizmor lane in `.github/workflows/ci.yml` audits `.github/` rather than
  `.github/workflows/`, so `dependabot.yml` and every composite action under
  `.github/actions/` are covered. A composite action runs with the calling
  workflow's permissions, so it is the same class of token-holding code.
  `CONTRIBUTING.md` and the pull-request template name the wider path too, so a
  local run still matches CI (#34).
- The Dependabot cooldown on the `github-actions` and `docker` ecosystems is
  7 days, up from 3, which is the floor zizmor's `dependabot-cooldown` audit
  enforces and matches the 7-day minor value already on `cargo`. Security
  updates are exempt from cooldown, so an advisory still arrives immediately.
  `docs/ci-cd.md` records why the earlier defence of the 3-day value did not
  hold and why no suppression was recorded (#34).
- `CONTRIBUTING.md` lists all six tier-1 gates `ci.yml` runs, in the same order
  and with the same flags. It named four, so a contributor who ran the listed
  commands could still fail CI on hadolint or the versions guard (#36).
- The licence of the project's own code and text is the Business Source
  License 1.1 (`LICENSE`, `NOTICE`): free for non-production use and for non-commercial production use, a commercial
  licence for any other production use, and Apache License 2.0 four years
  after each version. Every header, the README badge and licensing section,
  and the community and governance documents name it (#12).

[Unreleased]: https://github.com/rubentalstra/FerroBRIDGE/commits/main
