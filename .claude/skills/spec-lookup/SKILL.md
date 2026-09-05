---
name: spec-lookup
description: Look up the authoritative FHIR, FHIRconnect, OMOP, OMOCL, or openEHR ITS-REST requirement for any spec-facing behaviour, in the correct oracle order. Use before implementing or reviewing bridge behaviour, or to settle a "what does the spec say" question.
allowed-tools: Read, Grep, Glob, WebFetch
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Spec lookup

Answer a "what does the specification say" question about either target or the
openEHR side, and cite the source (document plus section). Never resolve a
spec-facing question from memory or from a reference implementation's behaviour
alone (`.claude/rules/spec-adherence.md`).

## Route by surface, then cite

| The question is about | The authority |
|---|---|
| what a FHIR mapping file may contain, and what it means | the FHIRconnect specification and its published JSON schemas |
| what an OMOP mapping file may contain, and what it means | the OMOCL specification |
| a FHIR resource shape, or a FHIR REST interaction | the HL7 FHIR specification for the version in play |
| an OMOP-side shape | the OMOP Common Data Model documentation |
| a call into an openEHR CDR, its status codes, headers, or error bodies | the openEHR ITS-REST specification |

Within a surface, the machine-readable artifact (a JSON schema, a FHIR
`StructureDefinition` or `OperationDefinition`) outranks a remembered summary,
and the normative prose gives it meaning. Read both where both exist.

The reference implementations (openFHIR for FHIRconnect, Eos for OMOCL,
EHRbase FHIR Bridge before them) and the reference deployments (FerroEHR,
FerroTERM) are behavioural evidence for spec-silent edge cases only. Cite them
explicitly as such and record the decision; never treat one as authority.

## Where to look

- **Vendored specifications (once they exist):** the research on issue #1 pins
  the versions, and the machine-readable artifacts are then vendored under
  `docs/specs/` or a codegen vendor directory, each with a `PROVENANCE.md`
  (`.claude/rules/vendored-inputs.md`). Grep there first, because it is the
  exact pinned text this project implements. **Nothing is vendored yet**, so
  today every lookup goes to the published sources below.
- **Published sources (fetch to confirm):**
  - FHIRconnect v1.0.0:
    <https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/index.html>
  - OMOCL: <https://github.com/SevKohler/OMOCL>
  - HL7 FHIR: <https://hl7.org/fhir/> (use the version-specific site for the
    version in play, never the "current" build for a released version)
  - OMOP Common Data Model: <https://ohdsi.github.io/CommonDataModel/>
  - openEHR ITS-REST:
    <https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/>
  - openEHR specifications index:
    <https://specifications.openehr.org/releases>

## How to answer

State the requirement, quote the decisive sentence, and cite the document plus
section (and the schema path or resource name where the machine-readable form
carries it). If the sources are silent, say so explicitly and name the
behaviour you would match; flag it as a spec-silent decision to record on the
tracker, never as a spec fact. If the question needs a version pin the research
has not made yet, say that too: an answer that assumes a version is worse than
no answer.
