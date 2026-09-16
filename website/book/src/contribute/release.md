<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Cutting a release

A milestone is a delivery promise, and a release is cut when its milestone
reaches zero open issues. This page is what happens around that cut: the
checklist before the tag, what the tag triggers, and what a consumer can check
afterwards. The repository-side copy is
[`docs/release.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/docs/release.md),
and the two say the same thing.

<!-- toc -->

## Before the tag

1. **The milestone is empty.** `gh issue list --milestone vX.Y.Z --state open`
   answers nothing, or the cut is called and the stragglers move to the
   next milestone.
2. **The product version moves in every file that declares it.** Today that is
   `CITATION.cff`, the product row of `docs/VERSIONS.md`, the root `Cargo.toml`
   `[workspace.package]` `version`, and the image tag default in
   `compose.yaml`. `scripts/checks/versions.sh` fails on any file left behind.
   This is the product line only; the library crates keep their own versions
   ([crate versions](crate-versions.md)).
3. **The changelog names the release.** `[Unreleased]` becomes the version and
   the date, with a fresh empty `[Unreleased]` above it and a new link
   reference. What sits under the version heading is what the release notes
   say, so read it as the release notes before you tag.
4. **The gates pass on the release commit.** The same set CI runs
   ([checks and gates](checks-and-gates.md)).

## The tag

The tag is signed and pushed from the working session that carried the
milestone, right after the version-bump pull request merges:

```bash
git tag -s vX.Y.Z -m "vX.Y.Z" <the merged release commit>
git push origin vX.Y.Z
```

The `release-tags` ruleset requires a signature on `refs/tags/v*`, so an
unsigned tag is refused at push time.

## What the tag triggers

`.github/workflows/release.yml` is dormant until a `v*` tag arrives. It does
not run on a push to `main`, on a pull request, or in a merge group.

```text
plan ── github-release (draft) ── build-binaries ── build-image ── finalize-release (publish) ── crates
```

- **plan** validates the tag shape, checks it against every file that declares
  the product version, and extracts the `## [X.Y.Z]` section of `CHANGELOG.md`
  as the release notes. A missing or empty section fails the release, so a cut
  can never ship with notes generated from the commit range. It also emits the
  target matrix the build jobs and the asset check both read, so the two cannot
  drift apart.
- **github-release** creates the release as a draft carrying those notes and
  attaches `compose.yaml`. A draft is mutable and invisible to anyone browsing
  releases, which is the window the asset uploads need.
- **build-binaries** calls the reusable `release-build.yml` once per target, on
  a runner of that target's own architecture, with no cache and with
  `cargo auditable`. Each call produces the tarball, its checksum, two SBOMs,
  three Sigstore bundles and the provenance envelope.
- **build-image** calls the reusable `release-image.yml` once the musl binaries
  exist, verifies them against the build lane's signer identity, and pushes
  `ghcr.io/rubentalstra/ferrobridge` for `linux/amd64` and `linux/arm64`.
- **finalize-release** checks that the draft carries every asset this version
  promises, then publishes. Publishing last means a half-assembled release is
  never visible. A pre-release publishes with `--latest=false`.
- **crates** uploads the `crates/*` members to crates.io once the release is
  public, so a refused upload never leaves a release half-cut. It runs in the
  `crates-io` environment, whose required reviewer pauses it until the owner
  approves, and authenticates with crates.io Trusted Publishing, so no
  long-lived registry token exists in the repository. Re-running the leg is
  safe: a version already on the index counts as done.

A second tag push queues behind the first. A release cancelled part-way through
publishing is worse than a slow one.

## What a consumer can check

**The release is frozen.** GitHub's repository-level immutable-releases setting
is on, so once a release is published its notes and its assets cannot be
edited.

**The tag is protected.** The `release-tags` ruleset blocks deletion and
non-fast-forward updates on `refs/tags/v*` and requires signatures, so a
published `vX.Y.Z` cannot be moved to another commit and cannot be deleted.
The commit a release names stays the commit it was cut from.

**Every artefact carries verifiable provenance.** Each tarball ships a Sigstore
bundle and a SLSA provenance envelope, and the image carries its attestations
as OCI referrers. Because the build runs in a reusable workflow, the signing
certificate names that workflow, so `gh attestation verify --signer-workflow`
pins the artefact to the release lane rather than to any workflow in the
repository. The commands are in `SECURITY.md`.

**The crates are verified after upload.** The publish script reads the registry
back rather than trusting the upload's exit status.

## A bad cut ships forward

**The no-retag rule is ours, not the platform's.** A bad cut becomes a new
patch version. Never move a tag, never delete a release and recreate it, and
never edit a published release's notes to fix what the changelog got wrong: fix
the changelog and cut the next version.

The lane enforces the half it can. `github-release` refuses to reopen an
already-published release for the same tag and fails with that message. The
rest is the ruleset and the immutable-releases setting above, which is why this
page states both rather than claiming the rule is enforced end to end.
