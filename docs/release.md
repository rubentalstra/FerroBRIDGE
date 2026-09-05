<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Cutting a release

A milestone is a delivery promise, and a release is cut when its milestone
reaches zero open issues. This page is the checklist the cut follows, in order,
and the record of what a published release is and is not protected against.

No specification governs this; it is FerroBRIDGE's own design. The lane it
describes is `.github/workflows/release.yml`, and the discipline every workflow
here obeys is `.claude/rules/ci-cd.md`.

## What the lane does

`release.yml` is dormant until a `v*` tag is pushed. It does not run on a push
to `main`, on a pull request, or in a merge group. `workflow_dispatch` re-runs
it for a tag that already exists and has to be dispatched at that tag.

```text
plan ── github-release (draft) ── build-binaries ── finalize-release (publish)
```

- **plan** validates the tag shape, refuses a dispatch that is not at the tag
  it names, checks the tag against every file that declares the product
  version, and extracts the `## [X.Y.Z]` section of `CHANGELOG.md` as the
  release notes. A missing or empty section fails the release, so a cut can
  never ship with notes generated from the commit range standing in for the
  changelog.
- **github-release** creates the release as a draft carrying those notes. A
  draft is mutable and invisible to anyone browsing releases, which is the
  window the asset uploads need.
- **build-binaries** is gated on a root `Cargo.toml`, the same detection
  `ci.yml` tier 2 uses. There is no Cargo workspace yet, so it reports
  `skipped` and a release with no binaries is a clean pass. It activates by
  itself when the workspace lands.
- **finalize-release** checks that the draft carries every asset this version
  promises, then publishes. Publishing last means a half-assembled release is
  never visible.

Concurrency is `cancel-in-progress: false`. A second tag push queues behind the
first, because a release cancelled part-way through publishing is worse than a
slow one.

## Before the tag

1. **The milestone is empty.** `gh issue list --milestone vX.Y.Z --state open`
   answers nothing, or the owner calls the cut and moves the stragglers to the
   next milestone.
2. **The version moves in every file the pin matrix names.** Today that is
   `CITATION.cff` and the product-version row of `docs/VERSIONS.md`; the root
   `Cargo.toml` `[workspace.package]` `version` joins them when the workspace
   lands. `scripts/checks/versions.sh` fails on any file left behind, and the
   `plan` job checks the same files against the tag.
3. **The changelog names the release.** `[Unreleased]` becomes the version and
   the date, with a fresh empty `[Unreleased]` above it and a new link
   reference. What sits under the version heading is what the release notes
   say, so read it as the release notes before you tag.
4. **The gates pass on the release commit.** The tier-1 set is
   `zizmor --min-severity=low .github/`, `actionlint`, `shellcheck
   --severity=style` over every tracked shell program, `hadolint` over every
   tracked Dockerfile, `scripts/checks/comment-style.sh --all`,
   `scripts/checks/versions.sh`, and `mdbook build website/book`. The Rust
   lanes join them when the workspace exists (`docs/ci-cd.md`).

## The tag

The signed tag is the owner's:

```sh
git tag -s vX.Y.Z -m "vX.Y.Z" <the merged release commit>
git push origin vX.Y.Z
```

The `release-tags` ruleset requires a signature on `refs/tags/v*`, so an
unsigned tag is refused at push time. `release.yml` takes it from there.

## After the tag

1. **Read the published release.** Its notes are the changelog section, and its
   asset list is what `finalize-release` verified.
2. **Post the board status update** with what shipped and what the next
   milestone targets (`.claude/rules/project-board.md`).

## What a published release is protected against

Two protections cover different things, and both are on.

**The release is frozen.** GitHub's repository-level immutable-releases setting
is enabled, so once a release is published its notes and its assets cannot be
edited. That is why the lane assembles a draft and publishes last: the draft is
the only window in which assets can still be attached.

**The tag is protected.** The `release-tags` ruleset is active on
`refs/tags/v*` and blocks `deletion` and `non_fast_forward` updates, and
requires signatures. A pushed `vX.Y.Z` cannot be moved to another commit and
cannot be deleted, so the commit a release names stays the commit it was cut
from.

Read the notes before you tag. Once the release publishes, the text you wrote
is the text that stands.

**The no-retag rule is ours, not the platform's.** A bad cut ships forward as a
new patch version. Never move a tag, never delete a release and recreate it,
and never edit a published release's notes to fix what the changelog got wrong;
fix the changelog and cut the next version. The lane enforces the half it can:
`github-release` refuses to reopen an already-published release for the same
tag, and fails with that message instead.

## Sources

- Available rules for rulesets:
  <https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets>
- Managing releases:
  <https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository>
- Automatically generated release notes:
  <https://docs.github.com/en/repositories/releasing-projects-on-github/automatically-generated-release-notes>
- GitHub Actions security hardening:
  <https://docs.github.com/en/actions/security-for-github-actions/security-hardening-for-github-actions>
