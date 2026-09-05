<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Build order

FerroBRIDGE has no release. This page says what exists in the repository today
and what gets built next, in order. The tracker is the scope: milestones are
releases, and a release is cut when its milestone has no open issue left.

## What exists today

- The design, recorded in
  [`docs/architecture.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/docs/architecture.md),
  with the pins and the decisions that follow from the primary sources.
- The pin matrix,
  [`docs/VERSIONS.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/docs/VERSIONS.md),
  and the guard that fails on cross-file version or licence drift.
- The working discipline: the engineering rules, the tracker workflow, and the
  committed check scripts.
- The security and analysis workflows that work on a repository with no code:
  OpenSSF Scorecard, CodeQL over the workflow files, and SonarQube Cloud's
  multi-language sweep.
- The release lane, which turns a signed `vX.Y.Z` tag into a GitHub release
  whose notes are the changelog section for that version. Its binary build is
  gated on the Cargo workspace and is skipped until that lands.
- This documentation site.

There is no Cargo workspace, no crate, no binary, and no container image.

## What comes next

The first code milestone is one verbatim round trip per target, on mapping
files published upstream rather than files written for the occasion:

1. The published `EVALUATION.problem_diagnosis.v1` FHIRconnect mapping, with an
   R4 `Condition` committed to a CDR over ITS-REST, read back, and mapped to
   FHIR again. The result has to equal the input except for the fields the
   specification defaults.
2. The published laboratory OMOCL files, emitting MEASUREMENT rows into a CDM
   5.4 database with a real Athena vocabulary loaded. Resolved `concept_id`
   values are asserted, and an unmapped code lands as `0` with its source value
   kept.

The Cargo workspace lands with that milestone, along with the server binary,
the container image, the attested binary build the release lane is already
gated for, and the conformance gate. Nothing is scaffolded before its issues
are filed.

## How correctness is measured

The published FHIRconnect and OMOCL mapping libraries get vendored verbatim by
committed fetch scripts with provenance, and exercised in full: every file
loads and validates, or carries a recorded skip with its reason. The composed
test stack runs an openEHR CDR reached over ITS-REST only, and a FHIR
terminology server. Both are configured deployments, never compile-time
dependencies. None of this exists yet; it lands with the first engine work.

## What is outside the design on purpose

Demographics, where FHIRconnect sends resources to an unspecified external
endpoint; FHIR Subscriptions, which the specification never mentions; a
materialised FHIR store, which would contradict the facade decision; and OMOP
CDM versions other than 5.4. Each one is a tracker issue rather than silence,
so you can read the reasoning and argue with it.

The current picture is the
[milestone list](https://github.com/rubentalstra/FerroBRIDGE/milestones), and
the [open issues](https://github.com/rubentalstra/FerroBRIDGE/issues) are the
worklist behind it.
