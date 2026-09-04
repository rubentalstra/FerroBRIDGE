---
name: memory-lives-in-repo
description: "Session learnings go in the repository's .claude/memory/ (tracked, shared by every session and contributor), never in a per-user directory; owner instruction 2026-09-04"
metadata:
  node_type: memory
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

Every memory (decision, owner preference, reference, correction) is a file in
`.claude/memory/` with an entry in `.claude/memory/MEMORY.md`, committed and
merged like any other change. The per-user auto-memory directory outside the
repository is not used for project learnings.

**Why:** the owner, 2026-09-04: "the memory needs to be in the project you
know that right!!! so everyone uses the same learning". A private note helps
one machine; a tracked file teaches every session and every contributor, and
it survives a crash with the repository.

**How to apply:** write the file with the frontmatter and SPDX header the
other files here carry, add its index line to `MEMORY.md`, and land it in the
current branch's pull request. Check for an existing file first and update it
rather than adding a duplicate.
