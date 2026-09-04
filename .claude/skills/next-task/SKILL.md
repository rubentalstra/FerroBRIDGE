---
name: next-task
description: Reads the tracker (GitHub Issues), picks the pinned or top open issue (or the issue the user names, skipping blocked ones), and restates it as a concrete in-session work plan naming the sources and files involved. Use when the user asks "what's next" or "what should I work on".
allowed-tools: Read, Grep, Glob, Bash
argument-hint: "[issue number] (optional)"
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# /next-task

Turns an open tracker issue into an actionable plan, the planning step of the
issue workflow (`.claude/rules/issue-workflow.md`). It does not do the work;
that is a separate step the caller takes after seeing the plan.

## Steps

1. **Read the tracker**: `gh issue list --state open` (the SessionStart dump
   annotates each issue with `{k/n}` sub-issue progress, `child-of #parent`,
   and open `BLOCKED-by`/`blocks` edges). Take the pinned issue (current focus)
   or the issue the user named. **Respect relationships**
   (`.claude/rules/issue-relationships.md`): do NOT pick an issue shown
   `BLOCKED-by` an open issue (surface its blocker as the real next task
   instead); for a parent issue, point at its next open child rather than the
   parent itself. Then `gh issue view <n> --comments` for the full contract
   (the opening summary plus `## Acceptance criteria`) and the running
   discussion, and `scripts/gh/rel.sh tree <n>` for its parent, children, and
   blockers.
2. **Turn the task into a plan**, stating:
   - **What** the task requires, in one or two sentences.
   - **Which sources or files** are involved. The project is in its design
     phase, so most issues are research: name the specification sections and
     the prior-art repositories to read, not files that do not exist. Once code
     exists, find the files by searching rather than guessing paths.
   - **Which mechanism** applies. A **research** issue produces cited evidence
     and a recommendation on the issue thread, and it may produce
     `docs/architecture.md` when issue #1 closes. An **implementation** issue
     on a generated layer changes the generator and regenerates, never a
     `// @generated` file (`.claude/rules/codegen.md`); an implementation issue
     on the engine is idiomatic Rust of our own design, built as compiling,
     tested increments.
   - **Which specifications govern it:** for any spec-facing task, name the
     authoritative source the work must be read against (HL7 FHIR for the
     version in play, FHIRconnect, the OMOP Common Data Model, OMOCL, or
     openEHR ITS-REST), per `.claude/rules/spec-adherence.md` and
     `/spec-lookup`. Doing the work starts by reading those.
   - **What "done" looks like** for this task specifically: the issue's
     `## Acceptance criteria` checklist, plus what proves it.
3. **When work on the picked issue actually starts** (the plan is accepted and
   the session proceeds), move it to `In Progress` on the public roadmap board:
   `scripts/gh/project.sh status <n> in-progress`, the one manual board move in
   the lifecycle (`.claude/rules/project-board.md`). If the session parks the
   issue unfinished, move it back (`scripts/gh/project.sh status <n> todo`); a
   stale In Progress column is a false public claim. If the board does not
   exist yet, the command fails loud; note that and continue.
4. **Do not edit the issue or commit:** recording progress happens after the
   work is actually done, not as part of planning it.
