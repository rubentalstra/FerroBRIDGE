---
paths:
  - ".github/**"
  - "scripts/**"
  - "sonar-project.properties"
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# CI/CD and supply-chain discipline

No specification governs this: our own design, grounded in the OWASP GitHub
Actions Security Cheat Sheet, SLSA v1.0, OpenSSF Scorecard, and Sigstore. The
repository is in its design phase, so this file carries only what applies now,
plus the shape the build and release lanes take when there is something to
build.

## What runs today

Three workflows, all of which work on a repository with no code:

- `.github/workflows/scorecard.yml`: OpenSSF Scorecard, an independent score of
  the repository's security posture, published to the OpenSSF API and to code
  scanning.
- `.github/workflows/codeql.yml`: CodeQL over the `actions` language, because
  the workflows in this directory are code that holds tokens. The Rust
  analysis is behind a detection job that looks for a root `Cargo.toml`, so it
  is skipped cleanly until the workspace lands and then activates by itself.
- `.github/workflows/sonar.yml`: SonarQube Cloud, the multi-language sweep
  over shell, YAML, and JSON. Advisory, gating no merge
  (`ai-code-review.md`); the Rust analysis and the coverage import join it
  with the workspace.

There is no build lane, no test lane, and no release lane, because there is
nothing to build. Do not add one before the workspace exists.

## Workflow security (every workflow, no exceptions)

- **Every `uses:` is pinned to a full commit SHA** with a trailing `# vX.Y.Z`
  comment. Dependabot (`github-actions`) bumps them. A tag or branch ref is a
  finding.
- **`permissions: {}` at workflow level**, with the minimum granted per job.
- **`persist-credentials: false`** on every `actions/checkout` that does not
  push with git.
- **No `${{ }}` context interpolation inside `run:`:** pass context through
  `env:`. This prevents template injection.
- **A publishing lane restores no build cache.** A cache an untrusted run could
  poison must not feed a release.

**Enforcement, stated honestly:** no lane runs `actionlint`, `zizmor`, or
`shellcheck` yet, because the CI workflow that would host them does not exist.
Until it does, run them by hand on any workflow or script you touch:
`actionlint`, `zizmor --min-severity=low .github/workflows`, and `shellcheck
--severity=style` over the tracked shell files. The lane that makes them
failing checks is added with the first CI workflow, and it goes in before the
Rust lanes, since it needs no Rust.

## Shell scripts are analysed like code

The tooling languages here are bash and Rust (`rust-style.md` §No Python).
Every committed shell script stays clean at `shellcheck --severity=style`, its
lowest floor, so every finding gates. A finding is FIXED, or it carries a
per-line `# shellcheck disable=SCnnnn` directive with its reason on the same
line. A blanket exclusion is refused, and no `.shellcheckrc` exists, because a
file that can turn a code off tree-wide eventually does.

## Rust CI lanes (activate with the Cargo workspace)

The lanes to stand up, in this order, with the local commands mirroring the CI
flags verbatim: `cargo fmt --all --check`; `cargo clippy --workspace
--all-targets --all-features -- -D warnings`; `cargo nextest run --workspace
--locked` plus `cargo test --doc --locked`; `cargo doc` with
`RUSTDOCFLAGS=-D warnings`; `cargo deny check` (advisories, licences, bans,
sources, which subsumes cargo-audit); MSRV via `cargo hack check
--rust-version`; `dependency-review-action` on pull requests; and the
`comment-style.sh` guard at `--all`. **Always `--locked`**, so CI fails on
lockfile drift rather than on registry drift. Commit `Cargo.lock`.

## Supply chain (the shape a release lane takes)

- **A release builds in a REUSABLE workflow** (`on: workflow_call`) so the
  builder is isolated and the signing identity is unreachable from build steps.
  That isolation is what makes the provenance non-falsifiable. Do not inline
  the build and attest steps back into a normal job.
- **Every release artifact carries provenance and a signed SBOM**, signed
  keyless through Sigstore (`id-token: write`), and consumers verify with
  `gh attestation verify … --signer-workflow …`.
- **Releases are immutable**: create the release as a draft, attach every
  asset, check the set is complete, and publish last, because the platform
  freezes a published release. The fix for a bad cut is a new patch version,
  never a retag.
- **A version pin has a single source of truth**, and a committed check fails
  on cross-file drift.

## Never

- Never unpin a `uses:` to a tag or branch, widen a job's permissions without
  cause, interpolate context into `run:`, or restore a cache in a publishing
  lane.
- Never weaken a gate to go green (`testing.md`); fix the cause.
- **Never add AI or Claude attribution** to any commit, PR, or release text.

## Official documentation (durable citations)

- OWASP GitHub Actions Security Cheat Sheet:
  <https://cheatsheetseries.owasp.org/cheatsheets/GitHub_Actions_Security_Cheat_Sheet.html>
- SLSA v1.0: <https://slsa.dev/spec/v1.0/>
- OpenSSF Scorecard: <https://github.com/ossf/scorecard-action>
- GitHub Actions security hardening:
  <https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions>
