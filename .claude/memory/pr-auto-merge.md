---
name: pr-auto-merge
description: "Every pull request gets auto-merge enabled the moment it is opened (gh pr merge <n> --auto --squash --delete-branch) so it lands when the conclusion check is green; owner instruction 2026-09-04"
metadata:
  node_type: memory
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
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
