---
name: merge-queue-signed
description: The main ruleset requires signed commits and an up-to-date branch, so pull requests merge one at a time after a LOCAL signed rebase; gh pr update-branch --rebase strips the signature and blocks the merge
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-13 five green pull requests with auto-merge armed sat unmerged
for hours. Two rules on `main` explain it: `required_status_checks` with the
strict up-to-date policy, so every merge makes the other branches stale and
auto-merge waits, and `required_signatures`, so a branch rewritten by
`gh pr update-branch --rebase` (a server-side rebase, which drops the
author's signature) is refused with "the base branch policy prohibits the
merge" even when every check is green.

**Why:** GitHub's auto-merge never updates a stale branch, and it cannot sign
a rebase with the author's key.

**How to apply:** drain open pull requests as a serial queue. For each one:
rebase LOCALLY onto `origin/HEAD` (commits are signed by `commit.gpgsign`),
push with `--force-with-lease`, let the `conclusion` check go green, and let
auto-merge fire before touching the next. Dependabot branches get the
`@dependabot rebase` comment (GitHub signs those). Never use
`gh pr update-branch --rebase`; the merge-commit form is signed by GitHub
but adds a merge commit to a squash-merged branch, so the local rebase is
the default. Never `--admin` without the owner asking. Opening a new pull
request while the queue drains makes every queued branch stale again.
