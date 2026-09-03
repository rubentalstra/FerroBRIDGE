<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Governance

How decisions get made in FerroBRIDGE, who makes them, and how that changes.

This document describes the project as it actually operates. Where the honest
description is "one person decides", it says so. A governance document that
describes a committee that does not meet is worse than none, because it invites
a reviewer to rely on a control that is not there.

## Current structure: benevolent dictator, one maintainer

FerroBRIDGE has a single maintainer ([MAINTAINERS.md](MAINTAINERS.md)) who
holds final say on every decision: what gets built, what gets merged, what gets
released, and what the project refuses to do. There is no steering committee,
no technical oversight body, no foundation, and no vote.

This is the standard structure for a project of this age and size, and it
carries the standard trade-off: decisions are fast and coherent, and the
project's resilience is one person's. The second half of that sentence is
treated as a finding rather than a footnote; see
[MAINTAINERS.md § If the maintainer is unavailable](MAINTAINERS.md#if-the-maintainer-is-unavailable).

## Where decisions are recorded

The tracker is the record. There is no separate design-document layer, and that
is deliberate: a decision record that outlives the code it justified becomes a
false authority, so this project keeps decisions in the places that cannot
drift out of sync with the tree
([`.claude/rules/issue-workflow.md`](.claude/rules/issue-workflow.md)).

| Kind of decision                   | Where it lives                                                                                     |
|------------------------------------|----------------------------------------------------------------------------------------------------|
| What to work on next               | a [GitHub issue](https://github.com/rubentalstra/FerroBRIDGE/issues); the open list is the worklist |
| Direction and status, publicly     | the roadmap project board, a view over the tracker (`.claude/rules/project-board.md`)              |
| Why a change looks the way it does | the pull request description that landed it, and the issue's closing comment                        |
| What a release contains            | [`CHANGELOG.md`](CHANGELOG.md) and the `vX.Y.Z` milestone                                          |
| The architecture                   | `docs/architecture.md`, once the research program on issue #1 produces it, and the `CLAUDE.md` files |
| What conformance means             | the tests, against the five specifications ([`.claude/rules/testing.md`](.claude/rules/testing.md)) |

Owner rulings and releases are milestones on the tracker. A decision that
exists only in a conversation is not a decision this project made.

## The design phase, and why nothing is built yet

FerroBRIDGE started as a research program rather than a codebase
([issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1)). The
foundation of a bridge is which specifications drive it and how mappings are
expressed, and getting that wrong is expensive to undo, so the evidence comes
first and the code follows. Until that program closes there are no crates and
no engine design, and the repository says so rather than describing a design it
has not chosen.

## How a change gets in

1. **An issue carries the contract**: what is wrong or missing, and the
   acceptance criteria that settle it.
2. **A pull request implements it** on a conventional-type branch, declaring
   `Closes #N`.
3. **The gates run.** They are not advisory and there is no override. Today
   that is the workflow and shell set (`actionlint`, `zizmor`, `shellcheck`)
   plus the comment-style guard; the Rust set (format, clippy at
   `-D warnings`, the test suite, the documentation build, `cargo deny`) joins
   with the workspace ([`.claude/rules/ci-cd.md`](.claude/rules/ci-cd.md)).
4. **The maintainer merges.** A pull request from an account without write
   access additionally requires a code-owner approval before it can merge
   ([`.github/CODEOWNERS`](.github/CODEOWNERS)).

**On required review, stated plainly.** The maintainer's own changes are not
independently reviewed by a second human, because there is no second human.
Requiring two approvals of oneself would be a control that reports "reviewed"
without anyone having reviewed, and this project would rather report the truth
and let a deployer weigh it. What stands in for review here is machine
enforcement: the gates above, and deterministic analysis on every pull request
(SonarQube Cloud and CodeQL) whose findings are advisory and never outrank the
specifications or these rules
([`.claude/rules/ai-code-review.md`](.claude/rules/ai-code-review.md)).

## The specifications are the authority, not the maintainer

The one place the maintainer explicitly does *not* have final say is
specification conformance. Five sources are the oracle for every wire-visible
behaviour: the HL7 FHIR specification for the version in play, FHIRconnect, the
OMOP Common Data Model, OMOCL, and openEHR ITS-REST. A conformance failure is
adjudicated against them, never against what the implementation happens to do
and never against another bridge's behaviour (openFHIR, Eos, and EHRbase FHIR
Bridge are prior art, so a bug in one is not a requirement). Where the
specifications are silent, the decision is the project's own and is labelled as
such wherever it is written down
([`.claude/rules/spec-adherence.md`](.claude/rules/spec-adherence.md)).

If you believe the implementation contradicts a specification, that is not a
matter of taste and it is not the maintainer's call: cite the section and open
an issue. Those are the reports this project most wants.

## Becoming a maintainer

The route is open and it is the ordinary one:

1. **Contribute.** Sustained, merged, self-directed work. The bar is the point
   at which review stops finding things, not a pull-request count.
2. **Show judgement in the areas that matter here.** The signal is
   specification discipline: reading the normative text first-hand, citing it,
   refusing to resolve a question from another implementation's behaviour, and
   being honest when a specification is silent.
3. **Ask, or be asked.** Either direction is normal. Open an issue, or say so
   on a pull request.

The maintainer decides, and says yes or no with a reason on the tracker rather
than by silence. A new maintainer receives write access, a row in
[MAINTAINERS.md](MAINTAINERS.md), and their handle in
[`.github/CODEOWNERS`](.github/CODEOWNERS) for the areas they own.

## What this project will not do

Recorded here so the questions do not have to be re-litigated in each pull
request:

- **No contributor licence agreement, and no copyright assignment.** You keep
  your copyright; the licence stays Apache 2.0 for everyone including the
  maintainer. This is a deliberate position, not an oversight.
- **No hand-coded mapping per FHIR resource or per OMOP table.** Mappings are
  specification-conformant YAML validated against the published schemas, which
  is the reason this project exists.
- **No re-modelling by hand of what a specification publishes in
  machine-readable form.** A change goes into the generator.
- **No compile-time dependency on a particular CDR.** The bridge reaches any
  openEHR CDR over ITS-REST, and FerroEHR is a reference rather than a
  requirement.
- **No patient data in the repository**, in a fixture, or in an issue.
- **No weakening a test, a gate, or an expectation to make a build green.** A
  red gate is information.
- **No claim the project cannot demonstrate.** If a claim has no evidence
  behind it, it does not get written. During the design phase that rule bites
  hardest: an undecided design is described as undecided.

## Code of conduct

[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) applies to every space this project
occupies. Enforcement is the maintainer's, at the contact route given there.

## Changing this document

Governance changes are pull requests against this file, like anything else, and
they take effect when they merge. If the structure described here stops being
true (a second maintainer joins, a legal entity forms, a decision body is
created), this file changes in the same pull request that makes it true, not
afterwards.
