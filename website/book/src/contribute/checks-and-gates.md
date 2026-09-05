<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Checks and gates

CI runs in two tiers. The first tier gates today, on a repository with no Rust
code: it covers the workflow files, the shell scripts, any Dockerfile, and the
committed guards. The second tier is the Rust set, written and gated behind a
job that looks for a root `Cargo.toml`, so it activates by itself when the
workspace lands. One `conclusion` job reads every result and is the single
required status check on `main`.

<!-- toc -->

## Tier 1, running now

| Check | Command | What it protects |
|---|---|---|
| Workflow security | `zizmor --min-severity=low .github/` | every `uses:` pinned to a commit SHA, no credential-persisting checkout, no context spliced into a shell, and the Dependabot cooldown window |
| Workflow correctness | `actionlint` | expressions that cannot evaluate, a `needs:` naming no job, unknown runner labels |
| Shell | `shellcheck --severity=style` over every tracked shell program | shellcheck's lowest floor, so every finding gates |
| Containers | `hadolint` over every tracked Dockerfile | container-recipe defects; no Dockerfile exists yet, so it reports that and passes |
| Comment style | `scripts/checks/comment-style.sh --all` | line comments only, `// TODO(#NNNN):` naming its issue, `// NOTE:` as a citation plus one sentence |
| Version drift | `scripts/checks/versions.sh` | every file that repeats a pin agrees with `docs/VERSIONS.md`, and no first-party file claims a licence other than BUSL-1.1 |
| The book | `mdbook build website/book` | a page that does not build fails the pull request |

Run the same commands locally before you push. A finding costs a local run
rather than a CI round trip.

## Tier 2, gated on the workspace

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo nextest run --workspace --locked` with
`cargo test --doc --locked`, `cargo doc` under `RUSTDOCFLAGS=-D warnings`,
`cargo deny check`, an MSRV check, and dependency review on pull requests.
Every cargo lane runs `--locked`, so CI fails on lockfile drift rather than on
registry drift.

## Workflow security rules

Every workflow follows the same four rules, and the analysers above check them:

- Every `uses:` is pinned to a full commit SHA with a trailing version comment.
- `permissions: {}` at workflow level, with the minimum granted per job.
- `persist-credentials: false` on every checkout that does not push with git.
- No `${{ }}` interpolation inside a `run:` block; context travels through
  `env:`.

## Advisory analysers

CodeQL scans the workflow files, because a workflow that holds a token is code.
SonarQube Cloud runs a multi-language sweep over shell, YAML, and JSON. OpenSSF
Scorecard scores the repository's security posture. All three are advisory:
they gate no merge, and a finding never outranks a specification citation or a
project rule. A wrong finding is recorded on the tracker rather than
suppressed quietly.

## Building this book

```console
$ cargo install mdbook mdbook-toc mdbook-mermaid
$ mdbook serve website/book
```

Use the versions pinned in `docs/VERSIONS.md`, which are the ones CI installs.
The site as GitHub Pages serves it, the landing page at the root and the book
under `/docs/`, is assembled by `scripts/site/assemble.sh _site`. That script
renders the roadmap block from the open milestones when `gh` is authenticated,
and leaves the block empty without failing when it is not.
