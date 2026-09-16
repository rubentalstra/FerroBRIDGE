---
name: upstream-reports-no-milestone
description: An upstream-report issue never carries a milestone; it is resolved by the specification's maintainers, not by a FerroBRIDGE release, so it would block every milestone it sat in
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-13 the seven `upstream-report` issues (#99 to #105) sat in the
v0.0.2 milestone. The owner: "all the upstream-report ones do never belong in
an milestone because they can not be solved right!!!"

**Why:** a milestone is a delivery promise this repository can keep. An
upstream report is closed when the specification or library it targets
changes, on a timeline nobody here controls, so a milestone holding one can
never reach zero open issues.

**How to apply:** file an `upstream-report` issue with labels only, never
`--milestone`. When one is found inside a milestone, take it out
(`gh issue edit <n> --milestone ""`). The FerroBRIDGE-side decision an
upstream defect forces (the adjudication, the refusal, the workaround
removal) is its own issue, and that one carries the milestone.
