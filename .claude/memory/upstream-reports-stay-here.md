---
name: upstream-reports-stay-here
description: An upstream-report issue is the record and stays in this tracker; nothing is filed on an external specification tracker, and no "owner action: file upstream" issue is ever created
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-13 issue #118 asked the owner to file the seven `upstream-report`
issues on the FHIRconnect, OMOCL, CDM, ITS-REST and Eos trackers. The owner
closed the idea: "we off course create the upstream report issues but we
will not report it upstream so these owner action issues please never make
that again ... but we keep creating these upstream issue reports".

**Why:** the value of a report is the cited record of the defect and of
FerroBRIDGE's own decision, which the tracker holds; filing on external
trackers is work the owner does not want to carry.

**How to apply:** keep writing `upstream-report` issues (the label, the
citations, what this implementation does, the resolution an upstream would
need), with no milestone ([[upstream-reports-no-milestone]]). Never create
an issue, checklist item or doc sentence that asks anyone to file them
externally, and never say a report "will be filed" or "has not yet been
filed" upstream. Prose says "recorded as an upstream-report issue".
