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

The Cargo workspace is a skeleton: the root manifests with the full lint set,
eleven placeholder library crates at version 0.0.0 that hold their names on
crates.io, a thin `ferrobridge` binary that does nothing yet, and a test
support crate. There is no container image and no release with a binary.

## What comes next

The program is one verbatim round trip per target, on mapping files published
upstream rather than files written for the occasion, in three releases:

1. **v0.0.2, the foundation.** The Cargo workspace with every lint, the vendored
   corpora with provenance, the two generated model crates (`fhir-types`,
   moved in from the sibling terminology server, and `omop-cdm`), the shared
   mapping foundation, the ITS-REST and terminology clients, the server binary,
   the container image, and the attested release lane with the crates.io leg.
   Nothing maps yet; everything the mapping needs exists and is published.
2. **v0.0.3, the FHIR round trip.** The published
   `EVALUATION.problem_diagnosis.v1` FHIRconnect mapping and its published
   extensions, with an R4 `Condition` committed to a CDR over ITS-REST, read
   back, and mapped to FHIR again. The result equals the input except for the
   set of fields the program declares defaulted or unmapped, and that set is
   asserted exactly.
3. **v0.0.4, the OMOP round trip.** The published laboratory OMOCL files,
   emitting MEASUREMENT and FACT_RELATIONSHIP rows into a CDM 5.4 database.
   Resolved `concept_id` values are asserted, an unmapped code lands as `0`
   with its source value kept and is counted, and a second run produces an
   identical database.

Later releases widen each side to the whole published library, then add FHIR
search, which no specification governs. Nothing is scaffolded before its
issues are filed.

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
