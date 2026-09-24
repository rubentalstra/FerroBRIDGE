---
name: deps-latest-sweep
description: Every work session sweeps the workspace dependencies to their latest crates.io releases and checks whether the sibling CDR split or added an openehr-* crate; owner instruction 2026-09-24
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-24 the owner asked, while the openEHR family was being bumped by
hand: "please also update all the crates to latest version please and also
understand if there are new crates like openehr-sdt because it's split off the
ITS crate". The same day the sibling had split `openehr-sdt` out of
`openehr-its` and published the family at 0.0.69.

**Why:** the openEHR crates are one lockstep line the sibling reshapes without
notice here, and a Dependabot bump per crate never builds alone, so the
sweep is a session task, not a bot's.

**How to apply:** at the start of a work session, and before a release cut,
compare every `[workspace.dependencies]` pin with crates.io (the crate's
`max_stable_version`), list the sibling's `crates/` directory against the
`openehr-*` set the workspace takes, and read the sibling's split or bump
commit for the module moves. Land the family together with its pin rows, the
fuzz manifest, both locks and `deny.toml`; fold any other outdated pin into the
same pull request. The docs the owner wants read are the current ones: the
pinned corpora are checked against their upstream heads at the same time.
