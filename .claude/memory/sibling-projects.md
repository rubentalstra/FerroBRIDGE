---
name: sibling-projects
description: "FerroEHR (../ferroehr) is the reference openEHR CDR and FerroTERM (../FerroTERM) the reference FHIR terminology server; both are read-only prior art from FerroBRIDGE and are never edited from this repository"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

FerroBRIDGE is the third project in the Ferro family, and the other two are
checked out beside it on the owner's machine:

- **FerroEHR** at `../ferroehr`: the openEHR CDR. It is the **reference CDR**
  for FerroBRIDGE, reached over the openEHR ITS-REST API, and never a
  compile-time dependency. Its root `CLAUDE.md` and `.claude/rules/` are the
  deeper reference for this repository's own discipline, since FerroBRIDGE's
  configuration descends from it through FerroTERM.
- **FerroTERM** at `../FerroTERM`: the FHIR terminology server. It is the
  **reference terminology server** for code systems and value sets, and the
  project this repository's working configuration was modelled on.

**How to apply:**

- Read either freely: their code, their rules, their git history, their closed
  issues. That is the fastest source of prior art for a decision here.
- **Never edit either from this repository.** No file changes, no commits, no
  branches in their repositories from a FerroBRIDGE session. A code change
  either project needs is made there, in its own session.
- **A tracker issue in a sibling MAY be filed from here when the owner asks.**
  Owner ruling 2026-09-05 ("you just create an issue directly there because
  it's my repo"): a request the bridge needs from a sibling (the first was
  FerroTERM #298, widening `fhir-types` to the full resource set) is filed in
  that repository's tracker with its labels and milestone, and recorded here
  on the FerroBRIDGE issue that depends on it. Issues only; code stays theirs.
- Neither is an oracle. A response from a running FerroEHR or FerroTERM is
  evidence in a comparison; the specification is the authority
  (`.claude/rules/spec-adherence.md`). A defect found in one of them is
  reported to that project rather than worked around here.
- All three are under the Business Source License 1.1 on the same terms
  since 2026-09-04 (see `license-busl.md`). Never copy code between the
  repositories anyway: each is its own Licensed Work with its own Licensor
  copy, and a licence decision is made per repository by the owner, never
  assumed from a sibling.
