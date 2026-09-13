<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Crate versions and the bump rule

The library crates under `crates/` are published on crates.io, and a published
version is immutable. That single fact produces one rule you have to follow in
every pull request that touches them. The two version lines it sits inside are
described on the
[version lines and releases](../operate/versions-and-releases.md) page.

<!-- toc -->

## The rule

**A pull request that changes any packaged content of a `crates/*` member bumps
that member's version in the same pull request.**

Packaged content is what the crate's `include` ships: `src/**`, `README.md`,
`LICENSE`, and `Cargo.toml`. A root `[workspace.dependencies]` entry the member
consumes counts too, because `cargo package` renders the concrete requirement
into the packaged manifest. Tests, benches, and `CLAUDE.md` are not packaged
and need no bump.

Three things move together:

1. the `version` in that member's `crates/*/Cargo.toml`,
2. any internal requirement naming it in the root `Cargo.toml`
   `[workspace.dependencies]` table,
3. `Cargo.lock`, refreshed with `cargo update -w` and committed.

A half-done bump fails the same way a missing one does.

## What is enforced, and where

| Where | What it does |
|---|---|
| `scripts/checks/crate-version-guard.sh` | compares a base ref with a head ref, and fails naming the member whose packaged content moved without its version |
| the `crate-version-guard` CI job | runs that script over the pull request's base and head; it is one of the jobs the `conclusion` check reads |
| `.claude/hooks/crate_version_bump_guard.sh` | runs the same script before a `git commit` or a `git push`, against the merge base with `origin/main`, and refuses the command with the findings |

The hook checks a commit against the working tree, because the change it is
about to record is not in `HEAD` yet, and a push against `HEAD`. It is the
early copy of the CI job, not a second rule.

Run the guard yourself the way CI does:

```bash
scripts/checks/crate-version-guard.sh origin/main HEAD
```

## The escape, and its one condition

A pull request carrying the `no-crate-bump` label skips the CI job. Use it only
when the diff provably alters no packaged bytes. The script itself reads no
labels, so the hook still reports; that is the intended asymmetry, since the
label is a reviewed decision rather than a local one.

## Two things that are normal

**A 0.0.0 version is a name reservation.** Every member except `fhir-types`
still holds its crates.io name at 0.0.0, and the guard says so rather than
demanding a bump: content under a reservation changes until the first real
version.

**A gap in the published sequence is fine.** Not every bumped version is
published. Publishing different content under a version that already exists is
the forbidden case, and crates.io refuses it anyway.

## `fhir-types` is generated

`crates/fhir-types/src/**` is emitted by `tools/fhir-codegen` and carries a
`@generated` banner. A change there is a generator change plus a regeneration,
never an edit of the output, and the bump follows the regeneration. The
`codegen-drift` CI job re-runs the emitter and fails on any difference.
