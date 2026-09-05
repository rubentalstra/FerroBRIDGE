<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# CI: the two tiers and the one required check

No specification governs this; it is FerroBRIDGE's own design, grounded in the
OWASP GitHub Actions Security Cheat Sheet, OpenSSF Scorecard, and the zizmor
audit set. The enforceable discipline is `.claude/rules/ci-cd.md`, which this
document does not repeat. What follows is the design of
`.github/workflows/ci.yml` and the reason it is shaped this way.

## The problem

The repository is in its design phase. There is no Cargo workspace, so a
conventional CI workflow would have nothing to run, and a workflow added later
would arrive after the files it is meant to guard. The workflows themselves
hold tokens, and the shell scripts under `scripts/` are the tracker helpers and
the committed guards, so both are live code from day one and both need a gate.

## The two tiers

`ci.yml` splits on whether a check needs Rust.

**Tier 1 runs today, on a tree with no code.**

| Job | Runs |
|---|---|
| `zizmor` | `zizmor --min-severity=low .github/workflows/`, with `GH_TOKEN` so the online audits (`impostor-commit`) work |
| `actionlint` | the official digest-pinned image, with `SHELLCHECK_OPTS='-e SC2016'` |
| `shellcheck` | `--severity=style` over every tracked `*.sh` and every tracked extensionless file with a shell shebang |
| `hadolint` | every tracked Dockerfile under `.hadolint.yaml` (`failure-threshold: warning`) |
| `comment-style` | `scripts/checks/comment-style.sh --all` |
| `versions` | `scripts/checks/versions.sh` |

Three of these report "nothing to check" on the current tree and gate from the
first matching file: `hadolint` has no Dockerfile, `comment-style` has no `.rs`
file, and `versions` skips the checks whose subject file is absent. They are in
place before the files they guard, which is the point.

**Tier 2 is written now and gated off.** A `detect` job checks out and looks
for a root `Cargo.toml`, publishing a boolean output. Every Rust job carries
`needs: detect` and `if: needs.detect.outputs.cargo == 'true'`: rustfmt,
clippy at `-D warnings`, nextest plus doctests, rustdoc at `-D warnings`,
`cargo deny check`, MSRV through `cargo hack check --rust-version`, and
`dependency-review-action` on pull requests. Each lane mirrors the local
command in `.claude/rules/ci-cd.md` verbatim. The workspace pull request
therefore changes nothing in CI; the lanes activate by themselves.

`hashFiles()` cannot do this work. It is evaluated before checkout, when the
workspace is empty, so a `if: hashFiles('Cargo.toml') != ''` gate never sees
the file. A detection job that checks out and tests for the file is the
correct primitive, and `codeql.yml` already uses the same one for its Rust
analysis.

## Why a mostly-skipped pipeline is still enforceable

A branch ruleset requires status checks by name, and a job that GitHub reports
as `skipped` satisfies a required check without having verified anything. Ten
skipped Rust lanes named individually in the ruleset would read as ten green
checks over a tree nobody compiled, and re-listing the checks would be an owner
action every time a job is added or renamed.

The `conclusion` job removes both problems. It `needs` every other job, runs
under `if: always()` so it reports even when its dependencies were skipped, and
reads `join(needs.*.result, ' ')` through `env:`. It fails when any result is
`failure` or `cancelled`, and passes when every result is `success` or
`skipped`.

**`conclusion` is the single required status check on `main`.** That gives one
name in the ruleset that never changes as jobs come and go, and it collapses
the whole matrix into one honest verdict: a skip is a skip because the gate
decided the lane had nothing to check, while a real failure anywhere still
fails the required check. A job added to `ci.yml` must be added to the
`conclusion` job's `needs` list in the same change, or its result is not
counted.

Reading the results through `env:` rather than splicing them into the `run:`
block is the same template-injection rule every other workflow follows
(`.claude/rules/ci-cd.md`).

## Configuration this workflow reads

- `.github/actionlint.yaml`: no self-hosted runner labels, and the
  configuration-variables check disabled.
- `.hadolint.yaml`: `failure-threshold: warning` plus the trusted registries.
- `.dockerignore`: denies everything but a staged `dist/` tree, so no source
  or build output enters a container build context.

Each tier-2 job installs the toolchain with a digest-pinned
`actions-rust-lang/setup-rust-toolchain` step of its own, which reads the
channel from `rust-toolchain.toml` when that file exists. Issue #20 lands the
workspace, the toolchain file, and a `./.github/actions/setup-rust` composite
action; each of those six steps carries a `TODO(#20)` marking the line to lift.

## Triggers and concurrency

`push` to `main`, `pull_request` against `main`, `merge_group`, and
`workflow_dispatch`. `cancel-in-progress` is true for pull requests only: a
push to `main` and a merge-group run each verify a commit that must keep its
own result, while a superseded pull-request run verifies a commit nobody will
merge.

## Owner actions (one-time, not scriptable)

These are repository settings only the owner can change. Issue #19 is the
checklist and the record; this table is the same list for the next reader, with
the state on 2026-09-05.

| Setting | State |
|---|---|
| `main` ruleset: requires a pull request, signed commits, and the `conclusion` status check with the strict up-to-date policy; deletions and non-fast-forward pushes blocked | done |
| Code scanning in advanced setup, with the CodeQL default setup off so `codeql.yml` is the analysis path | done |
| Secret scanning with push protection, Dependabot alerts, and Dependabot security updates | done |
| Artifact attestations, for the release lane when it lands | done |
| The `SONAR_TOKEN` secret, with SonarQube Cloud's Automatic Analysis off (`.claude/rules/ai-code-review.md`) | done |
| Pages publishes from GitHub Actions and serves `ferrobridge.eu` with HTTPS enforced; the apex A records point at the four GitHub Pages addresses, `www` is a CNAME to `rubentalstra.github.io`, and the domain is verified for the account | done |
| The roadmap board and the label bootstrap (`scripts/gh/labels.sh`) | done |
| Registration at bestpractices.dev, with the returned badge added to the README | open |
| A `crates-io` environment with a required reviewer | conditional: only if FerroBRIDGE ever publishes a crate. `docs/architecture.md` section 7 has it consuming published crates rather than publishing its own |

`conclusion` is the contract for the required-checks list. Add no other CI check
to it: a job added to `ci.yml` joins the `conclusion` job's `needs` list
instead, which keeps the ruleset stable as the pipeline grows.

## Sources

- GitHub Actions security hardening:
  <https://docs.github.com/en/actions/security-for-github-actions/security-hardening-for-github-actions>
- OWASP GitHub Actions Security Cheat Sheet:
  <https://cheatsheetseries.owasp.org/cheatsheets/GitHub_Actions_Security_Cheat_Sheet.html>
- zizmor: <https://docs.zizmor.sh/>
- actionlint: <https://github.com/rhysd/actionlint>
- hadolint: <https://github.com/hadolint/hadolint>
- Required status checks and rulesets:
  <https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets>
