---
paths: ["**/*.rs", "scripts/**", "docs/**"]
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Spec adherence (the published specifications are the oracle)

The conformance authority for this project is the published specifications, not
another implementation, not memory, not intuition. FerroBRIDGE sits between
openEHR and two targets, so it answers to five sources:

1. **HL7 FHIR** (<https://hl7.org/fhir/>), for the version in play: every
   FHIR-side shape and every FHIR REST interaction the bridge exposes or
   consumes.
2. **FHIRconnect**
   (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/index.html>),
   the FHIR mapping specification: what a model-mapping or contextual-mapping
   file may contain, and what it means. Its published JSON schemas are the
   machine-readable half of that authority.
3. **The OMOP Common Data Model**
   (<https://ohdsi.github.io/CommonDataModel/>): every OMOP-side shape.
4. **OMOCL** (<https://github.com/SevKohler/OMOCL>), the OMOP mapping
   specification.
5. **openEHR ITS-REST**
   (<https://specifications.openehr.org/releases/ITS-REST/latest/>): every call
   the bridge makes into a CDR, with its status codes, headers, and error
   bodies.

The precise release of each, and which parts are vendored in-tree, are output
of the research on issue #1. Once versions are pinned, the machine-readable
artifacts (the FHIR packages, the mapping schemas) are vendored under
`docs/specs/` or a codegen vendor directory with a `PROVENANCE.md` per tree
(`vendored-inputs.md`), and this file gains the exact paths. Until then, read
the published sources at the URLs above and cite them.

## Hard rules

- **Before implementing or changing any spec-facing behaviour, read the
  governing section first.** Route by surface:
  - a mapping file's grammar or semantics goes to FHIRconnect or OMOCL,
    whichever governs that target, plus its schema;
  - a FHIR resource shape or REST interaction goes to the FHIR specification
    for the version in play;
  - an OMOP-side shape goes to the OMOP Common Data Model;
  - a call into a CDR goes to openEHR ITS-REST.
- **NEVER LAX. Strictness is a hard rule.** The bridge accepts EXACTLY what the
  governing specification admits, nothing more and nothing less.
  - Everything a specification REFUSES, we refuse, and every refusal is an
    ASSERTED NEGATIVE TEST pinning its error outcome (an invalid mapping file,
    an unknown field, input a mapping does not cover), so a silently loosened
    reader is a failing build rather than quiet drift.
  - A spec-SILENT form is accepted only with a first-hand citation, recorded on
    a tracker issue. Stalled or contradictory upstream material is never
    carried silently.
  - Weakening any existing refusal needs a spec-grounded adjudication recorded
    on an issue, with the flipped test updated to assert the new expected
    outcome. Inventing a prohibition the specification does not contain is the
    same defect class as leniency: strict means exact, in both directions.
- **Cite the source.** A conformance-relevant decision names the specification
  and section in the commit or PR description. A deliberate deviation or gap
  gets a `// NOTE:` with the reference and the reason.
- **Cite ONLY durable references, never an internal markdown file as a design
  authority.** In code, doc comments, and findings, justify behaviour by citing
  one of the five sources above or official external documentation (the Rust
  book and reference, a pinned crate's docs.rs page). An internal plan document
  is deleted in the PR that implements it and is never a citable authority; the
  durable record is the closed issues, PR descriptions, `CHANGELOG.md`, git
  history, and the living reference docs. Where the specifications are SILENT
  (the process model, storage mechanics, transport details, infrastructure),
  flag it explicitly: "no specification governs this: our own design".
- **The reference implementations are prior art, never a substitute for the
  specification.** openFHIR implements FHIRconnect, Eos implements OMOCL, and
  EHRbase FHIR Bridge preceded both. Read them for how a problem was solved.
  Where one disagrees with the specification text, the specification wins and
  the divergence is worth a note. Never resolve a spec question from an
  implementation's observed behaviour alone.
- **FerroEHR and FerroTERM are reference deployments, not oracles.** A response
  from either is evidence in a comparison, never the reference. A divergence
  found against a reference server is attributed against the specification
  before anything is changed, and if the defect is theirs it is reported to
  that project rather than worked around here.
- **A defect in a published specification is reported outbound.** File it as an
  `upstream-report` issue (`issue-workflow.md`) with what the specification
  says, what this implementation does, and the resolution sought. Do not encode
  a workaround with no record.
- Subagents doing spec-facing work must be handed the relevant sections or URLs
  in their prompt, and reviewers verify claims against them.

## Two targets, one discipline

The FHIR side and the OMOP side are separate conformance surfaces with separate
oracles, and neither one's answer settles the other. A single code path may
serve both only where the specifications genuinely agree, and any difference
they express is a real difference in the code, never a shared approximation
that is wrong on one side. State per-target and per-version differences
explicitly wherever they appear.

## Make no claim beyond the specification

While the project is in its design phase, the strongest temptation is to state
a technical fact about a target from memory. Do not. Every claim about
FHIRconnect, FHIR, OMOP, OMOCL, or openEHR that appears in this repository is
one the product statement in `CLAUDE.md` already makes, or one the research has
established with a citation. Anything else is a question for the research
program, not a sentence in a file.
