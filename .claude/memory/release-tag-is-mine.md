---
name: release-tag-is-mine
description: The release tag (vX.Y.Z and the -rc pre-releases) is always pushed by the working session, never handed to the owner as an action; every cut is run end to end from here
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-13 a pull request body told the owner which `git tag -s` and
`git push origin v0.0.2-rc.1` commands to run. The owner: "never do this
okay!! ... you always run the tag never say that i need to do that".

**Why:** the owner wants the release cut, pre-release rehearsals included,
finished from the session like every other step of a milestone; a hand-back
is unfinished work.

**How to apply:** a release cut is: the version bump pull request
(`Cargo.toml`, `CITATION.cff`, `docs/VERSIONS.md`, `compose.yaml`, the
`CHANGELOG.md` section), its merge, then `git tag -s vX.Y.Z -m vX.Y.Z` and
`git push origin vX.Y.Z` from the session, then reading the release run and
fixing what fails. A change to the lane itself is rehearsed with a
`-rc.N` pre-release tag first, also pushed from here. Docs and PR bodies
never contain "the owner runs" for a tag; they describe what the cut does.
