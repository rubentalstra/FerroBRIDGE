---
name: pr-auto-merge
description: "Every pull request gets auto-merge enabled the moment it is opened (gh pr merge <n> --auto --squash --delete-branch) so it lands when the conclusion check is green; owner instruction 2026-09-04"
metadata:
  node_type: memory
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

After opening a pull request, enable auto-merge at once:

```sh
gh pr create ... && gh pr merge <n> --auto --squash --delete-branch
```

Never leave a pull request waiting for a manual merge.

**Why:** the owner asked on 2026-09-04: "for PR's do not forget to trigger
auto merge okay!! so when the CI is green it will be merged". The `main`
ruleset requires the `conclusion` status check, so auto-merge is the correct
hand-off: GitHub merges the moment the check passes. The repository has
`allow_auto_merge` and `delete_branch_on_merge` on.

**How to apply:** until `ci.yml` (#16) exists the required `conclusion` check
never reports, so an armed auto-merge waits. In that window say so in the
hand-off and let the owner merge through the admin bypass; never use
`--admin` without asking.

**Inside a worktree:** when auto-merge fires while the command is still
running, `gh pr merge --auto --squash --delete-branch` ends with `fatal:
'main' is already used by worktree at …`, because its final local cleanup
tries to check out `main`, which the primary checkout holds. The merge and
the remote branch deletion already succeeded; confirm with `gh pr view
--json state,mergedAt` and never read that line as a failed merge. After the
merge the orchestrator removes the worktree (`git worktree remove --force`),
an owner instruction from 2026-09-12.
**Local branches:** a merged pull request leaves its local branch behind,
and a worker's `worktree-agent-*` branch stays after its worktree is
removed. The owner on 2026-09-13: "you need to delete the local branch if
it's done right now it's an very very big mess". After every merge run
`git fetch --prune`, delete every local branch whose upstream is `[gone]`
and every `worktree-agent-*` branch, and keep only `main` plus branches
with an open pull request. Every branch with work on it is pushed the
moment it has a commit; nothing lives only locally.

**Branch from `origin/HEAD`, never from the local `main`:** on 2026-09-13 a
fix branch was cut from a local `main` that was several merges behind, and
a `git reset --soft origin/HEAD` on it turned the stale working tree into a
commit that deleted the very workflow it meant to edit. Every new branch
starts with `git fetch origin && git checkout -b <type>/<slug> origin/HEAD`,
and a commit is made with named paths (`git add <files>`), never `git add
-A` from the root after a local build in an excluded crate: the fuzz build
tree (a gigabyte of objects) was swept into a commit that way and GitHub
refused the push on file size.
