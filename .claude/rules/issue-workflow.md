<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Issue workflow (the tracker loop)

**The tracker is GitHub Issues: the open issue list IS the worklist.** Issue
state is edited only via `gh`; never track work only in chat. This file is the
loop, the label taxonomy, and the cadence. Relationships between issues live in
`issue-relationships.md`; the public board is `project-board.md`.

## The loop

1. **Orient.** `gh issue list --state open` (the SessionStart hook injects it,
   annotated with each issue's sub-issue progress `{k/n}`, `child-of #parent`,
   and open `BLOCKED-by`/`blocks` edges). Pick the pinned issue (pins are the
   current focus, max 3) or the issue the user names; **skip an issue shown
   `BLOCKED-by` an open issue** (work its blocker first) and prefer the next
   open child of a parent. Read the contract with `gh issue view <n>
   --comments` and its relationship graph with `scripts/gh/rel.sh tree <n>`.
2. **Read the contract.** The issue body opens with a plain summary (no
   heading: what and why, decisions, spec citations), followed by an
   `## Acceptance criteria` checklist and an optional `## Tasks` task list. New
   work discovered en route gets its own issue (`gh issue create`) that is then
   **linked** with `scripts/gh/rel.sh`, as a sub-issue of the issue it
   decomposes or a `blocked-by`/`blocking` dependency for real sequencing,
   never a prose "see also" deferral.
3. **Do the work.** At pickup, move the issue to `In Progress` on the board
   (`scripts/gh/project.sh status <n> in-progress`). First read the governing
   spec text (`/spec-lookup`; FHIR, FHIRconnect, OMOP, OMOCL, and openEHR
   ITS-REST are the oracles, see `spec-adherence.md`). While the project is in
   its design phase, most issues are research: the deliverable is cited
   evidence and a recommendation rather than code (`CLAUDE.md` §Status). Once
   the architecture is fixed, a generated layer changes through its generator
   (never a hand-edit of `// @generated`) and the engine is idiomatic Rust of
   our own design, built as compiling, tested increments.
4. **Record progress on the issue.** Tick verified acceptance-criteria
   checkboxes (`gh issue edit <n>`), and post substantive status or decisions
   as comments (`gh issue comment <n>`); the issue thread is the durable
   record.
5. **Commit on a conventional-type branch** (see the branch rule below) with a
   descriptive subject; the PR body declares `Closes #<n>` so the merge into
   `main` auto-closes the issue (never close by hand when a PR carries the
   work). One `Closes` keyword closes one issue: "Closes #1, #2" closes only
   #1, so repeat the keyword per issue and verify after the merge.
6. **Close out** with `/phase-done`: verify the acceptance criteria are met,
   write the close narrative into the PR description, and post the handoff
   comment on the issue.

## Label taxonomy (industry-standard, nothing invented)

Bootstrap the labels once with `scripts/gh/labels.sh`.

- **Type:** exactly ONE per issue, mapped to the conventional-commit types:
  `bug` maps to fix, `enhancement` to feat, `documentation` to docs, plus
  `chore`, `refactor`, `perf`, `test`, and `ci`.
- **Priority:** `P0` (critical, drop everything), `P1` (high, current focus),
  `P2` (normal), `P3` (backlog).
- **Domain or area:** `spec:FHIR` (the HL7 FHIR wire), `spec:FHIRconnect` (the
  FHIR mapping specification and its JSON schemas), `spec:openEHR` (the openEHR
  ITS-REST wire), `spec:OMOP` (the OMOP Common Data Model target),
  `spec:OMOCL` (the OMOP mapping specification), `research` (a design-phase
  investigation). Add more as the
  project grows; keep the set small and meaningful.
- **Outbound:** `upstream-report` for a report of a defect, contradiction, or
  silence in a published specification. The issue IS the report: it opens with
  a plain summary, then what the specification says (with citations), what this
  implementation does, and the resolution sought upstream.

## Milestones = releases

A milestone is a delivery promise (`vX.Y.Z`). A release is cut when its
milestone reaches zero open issues (or the owner calls the cut and moves the
stragglers to the next milestone). Every en-route issue goes in the CURRENT
milestone, never the next. A new `vX.Y.Z` milestone gets a due date (the
board's Roadmap view places items by it, via `scripts/gh/project.sh
sync-dates`).

## The fix-first cadence

**An issue filed while working a unit is FIXED before the next unit starts.**
Section findings, verification-pass findings, guard gaps, and en-route defects:
filing the issue is the record, never permission to move on. Tractable findings
are fixed in the same branch; genuinely separable work becomes a linked issue
that is closed before the current program advances.

## Branches use conventional types

`<type>/<kebab-case-slug>` with `type` in `feat`, `fix`, `chore`, `docs`,
`refactor`, `perf`, `test`, `ci`, `build`, `release` (the Conventional Commits
type set): for example `feat/mapping-loader`, `fix/its-rest-status-mapping`.
Pick the type by the dominant change; an issue's branch is normally
`feat/<issue-slug>` mirroring the issue's type label. Never force-push `main`.

## Durable versus session-scoped

Issues and git survive `/clear` and `/compact`; the built-in todo tool is
session-scoped, so the tracker is the durable layer. Record progress (tick
checkboxes, comment, open issues for new work) and commit before ending a
session.

## Never add AI or Claude attribution

Commit messages, PR text, issue and comment bodies describe only the change
itself: no `Co-Authored-By: Claude`, no "Generated with", no bot trailer or
footer, ever. This is an absolute rule with no exceptions.
