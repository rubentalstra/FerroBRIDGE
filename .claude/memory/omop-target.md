---
name: omop-target
description: "Owner confirmation 2026-09-03: OMOP is a first-class target beside FHIR, driven by the OMOCL specification with Eos as its reference implementation; FerroEHR #2652, now FerroBRIDGE #2, is the origin of the question"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On **2026-09-03** the owner confirmed that FerroBRIDGE has **two first-class
targets**, not one: HL7 FHIR and the **OMOP Common Data Model**. The scope
widened during the repository's own bootstrap, so any file written before that
moment named FHIR alone and was corrected in the same session.

**The two targets follow two different mapping specifications**, and this is
the correction that matters most:

- **FHIRconnect** is the FHIR mapping specification. Its reference
  implementation is **openFHIR**. It is FHIR-only.
- **OMOCL** (<https://github.com/SevKohler/OMOCL>) is the OMOP mapping
  specification. Its reference implementation is **Eos**.

So there is no single mapping specification covering both targets, and treating
FHIRconnect as if it covered OMOP is a factual error to catch in review.

**Origin:** the OMOP question came from FerroEHR issue **#2652**, carried into
this repository as **FerroBRIDGE #2**.

Update 2026-09-03 (research closed): `docs/architecture.md` exists; pins are FHIRconnect v1.0.0, FHIR R4, OMOCL v1.0.0, OMOP CDM v5.4, ITS-REST 1.1.0; one shared foundation, two interpreters, two sinks; FHIR facade + OMOP batch ETL; the FHIR model is a dependency, the CDM types are generated from the OHDSI definitions.

**How to apply:**

- Make **no technical claim about OMOP beyond the product statement**
  (`product-scope.md`). The CDM version, the shape of the openEHR to OMOP path,
  the ETL question, and how vocabularies are resolved are the research
  program's to settle under issue #1. Do not fill any of that in from memory.
- Both targets are separate conformance surfaces with separate oracles, and
  neither one's answer settles the other
  (`.claude/rules/spec-adherence.md` §Two targets, one discipline).
- The tracker carries `spec:FHIR`, `spec:FHIRconnect`, `spec:OMOP`,
  `spec:OMOCL`, and `spec:openEHR` labels so a finding lands on the right
  surface.
