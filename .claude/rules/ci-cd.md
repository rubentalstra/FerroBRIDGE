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

Six workflows, all of which work on a repository with no code:

- `.github/workflows/ci.yml`: the two-tier gate. Tier 1 runs now (zizmor,
  actionlint, shellcheck, hadolint, the comment-style guard, the versions
  guard); tier 2 is the Rust set, gated behind a `detect` job that looks for a
  root `Cargo.toml`. The `conclusion` job is the single required status check
  on `main`. The design is `docs/ci-cd.md`.
- `.github/workflows/scorecard.yml`: OpenSSF Scorecard, an independent score of
  the repository's security posture, published to the OpenSSF API and to code
  scanning.
- `.github/workflows/codeql.yml`: CodeQL over the `actions` language, because
  the workflows in this directory are code that holds tokens. The Rust
  analysis is behind a detection job that looks for a root `Cargo.toml`, so it
  is skipped cleanly until the workspace lands and then activates by itself.
- `.github/workflows/sonar.yml`: SonarQube Cloud, the multi-language sweep
  over shell, YAML, and JSON. Advisory, gating no merge
  (`ai-code-review.md`). The instrumented coverage run and the
  `sonar.projectVersion` derivation sit behind a `hashFiles('Cargo.toml')`
  step gate and start reporting when the workspace lands.
- `.github/workflows/docs.yml`: the documentation site. It builds the mdBook on
  every pull request and deploys from `main` through GitHub Pages, using the
  pinned toolchain in `.github/actions/docs-toolchain` (#18).
- `.github/workflows/release.yml`: the release lane, dormant until a `v*` tag
  is pushed. It validates the tag, checks it against every file that declares
  the product version, takes the release notes from the matching
  `CHANGELOG.md` section, creates the release as a draft, and publishes only
  after the expected asset set is complete. Its binary lane sits behind the
  same root-`Cargo.toml` detection and is skipped until the workspace lands.
  The checklist a cut follows is `docs/release.md` (#63).

The Rust lanes in `ci.yml`, `codeql.yml` and `sonar.yml` are written and gated
off, so they need no edit when the workspace lands. `.github/dependabot.yml`
carries the same property: its `cargo` and `docker` entries are inert until
their manifests exist, and so does the binary lane of `release.yml`.
`.github/release.yml` is a different file from the workflow: it configures
GitHub's auto-generated release notes, which the lane never uses, because a
release ships the hand-curated changelog section or it fails.

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

**Enforcement:** `ci.yml` tier 1 runs `zizmor --min-severity=low .github/`,
`actionlint`, and `shellcheck --severity=style` on every push to `main`, pull
request, and merge-group run. Run the same three by hand before pushing a
workflow or script change, so a finding costs a local run rather than a CI
round trip. The zizmor path is the whole of `.github`, so `dependabot.yml` and
every composite action under `.github/actions/` are audited alongside the
workflows. Never narrow it back to make a finding disappear: fix the cause, or
record a `# zizmor: ignore[audit]` suppression with its reason on the line the
finding names.

## Shell scripts are analysed like code

The tooling languages here are bash and Rust (`rust-style.md` §No Python).
Every committed shell script stays clean at `shellcheck --severity=style`, its
lowest floor, so every finding gates. A finding is FIXED, or it carries a
per-line `# shellcheck disable=SCnnnn` directive with its reason on the same
line. A blanket exclusion is refused, and no `.shellcheckrc` exists, because a
file that can turn a code off tree-wide eventually does.

## Rust CI lanes (gated in `ci.yml`, activate with the Cargo workspace)

The lanes, with the local commands mirroring the CI
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
- **A release is assembled as a draft and published last**: create the draft,
  attach every asset, check the set is complete, then publish, so a
  half-assembled release is never visible. The fix for a bad cut is a new patch
  version, never a retag, and both halves are enforced: the immutable-releases
  setting freezes a published release's notes and assets, and the
  `release-tags` ruleset stops the tag being moved or deleted
  (`docs/release.md`).
- **A version pin has a single source of truth**, and a committed check fails
  on cross-file drift.
- **The library crates publish to crates.io through Trusted Publishing** (OIDC,
  no long-lived token), in dependency order, from a dispatch lane and from the
  release lane, with a dry run on every pull request and the codegen drift gates
  ahead of it, so a published generated crate never disagrees with its
  generator.

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
