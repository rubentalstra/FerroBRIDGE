<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# How the work is organised

The tracker is GitHub Issues, and the open issue list is the worklist. There is
no separate plan document, because a plan document rots and an issue does not.
This page tells you how to find work, how a change travels, and what a
contribution needs to carry.

<!-- toc -->

## The loop

1. Read the open issues. Each one opens with a plain summary, then an
   acceptance-criteria checklist, and sometimes a task list.
2. Pick one that no open issue blocks. Blocked issues carry a dependency edge,
   so you can see the blocker before you start.
3. Read the governing specification section first. FHIR, FHIRconnect, OMOP CDM,
   OMOCL, and openEHR ITS-REST are the authority, in that order of relevance to
   the change at hand.
4. Work on a branch named `<type>/<slug>`, with the type from the conventional
   commit set: `feat`, `fix`, `chore`, `docs`, `refactor`, `perf`, `test`,
   `ci`, `build`, or `release`.
5. Open a pull request whose body declares `Closes #<n>`, so the merge closes
   the issue.

Labels carry the type and the priority. Milestones carry the release, and a
release is cut when its milestone has no open issue left.

## What makes a change reviewable

- **A specification citation for anything spec-facing.** Name the document and
  the section. Memory is not a source, and neither is another implementation's
  behaviour.
- **An explicit flag where no specification governs the decision.** Write it
  plainly: this is FerroBRIDGE's own design. FHIR search, identity, failure
  policy, and extension ordering are all in that category.
- **A changelog entry** under `[Unreleased]` for anything with a user-visible
  effect. The format is Keep a Changelog 1.1.0.
- **Tests that measure against the specification.** Never adjust an expectation
  to match a bug, and never weaken or skip a test to get a build green.

## What is most useful right now

The project is in its design phase, so evidence beats code. A specification
citation that contradicts a decision on record, a measurement, or first-hand
experience with FHIRconnect, OMOCL, or an openEHR CDR in production is worth
more than a speculative implementation.

## Licence of contributions

Inbound equals outbound: your contribution is licensed under the same Business
Source License 1.1 that covers the project. There is no contributor licence
agreement and no copyright assignment.

The rules in full are in
[`CONTRIBUTING.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/CONTRIBUTING.md)
and
[`CLAUDE.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/CLAUDE.md).
