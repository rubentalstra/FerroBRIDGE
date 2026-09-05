---
name: milestones-0-0-x
description: "Release milestones start at v0.0.1 and step by 0.0.x (owner, 2026-09-04), never a v0.1.0 opener; v0.0.1 is the pre-code setup, v0.0.2 the workspace and first round trip"
metadata:
  node_type: memory
  type: feedback
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

Milestones start at `v0.0.1` and step `v0.0.2`, `v0.0.3`, never a `v0.1.0`
opener. Set on 2026-09-04 when a first `v0.1.0` milestone was renamed at the
owner's request.

**Why:** the Ferro family runs an `0.0.x` line while pre-release (FerroTERM's
workspace is at 0.0.9), and the owner wants small, frequent cuts.

**How to apply:** every en-route issue goes in the current `0.0.x` milestone
(`.claude/rules/issue-workflow.md`); a new milestone is the next patch number.
Milestones have no due dates yet, so `scripts/gh/project.sh sync-dates` has
nothing to place until the owner sets them.

Update 2026-09-05 (owner steer for the architecture pass): the round trips
are split by target. `v0.0.2` holds the workspace, the shared foundation, the
server shape, the container and the release lane; `v0.0.3` is the FHIR round
trip on the published `EVALUATION.problem_diagnosis.v1` mapping; `v0.0.4` is
the OMOP round trip on the published laboratory OMOCL files. A plan laid out
deliberately places each issue in the milestone its build order implies; the
"current milestone" rule is for work found while implementing.
