---
name: sibling-projects
description: "FerroEHR (../ferroehr) is the reference openEHR CDR and FerroTERM (../FerroTERM) the reference FHIR terminology server; both are read-only prior art from FerroBRIDGE and are never edited from this repository"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

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
  branches, no tracker writes in their repositories from a FerroBRIDGE
  session. A change either project needs is raised there, in its own session,
  through its own tracker.
- Neither is an oracle. A response from a running FerroEHR or FerroTERM is
  evidence in a comparison; the specification is the authority
  (`.claude/rules/spec-adherence.md`). A defect found in one of them is
  reported to that project rather than worked around here.
- Their licences differ from this one (both moved to BUSL 1.1 on 2026-09-03,
  while FerroBRIDGE stays Apache 2.0). Never copy code between the
  repositories on the assumption the terms match, and never copy a licence
  decision across (see `license-apache.md`).
