---
name: legacy-standards-first-class
description: The owner wants FerroBRIDGE to serve the systems still on older interoperability standards (HL7 v2.x first) so the bridge spans the oldest wire formats to the newest; legacy support is a first-class goal built in from the start, not a later add-on; owner 2026-09-25, research on #248
metadata:
  type: feedback
---

<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

On 2026-09-25 the owner: "we also need to support legacy systems please later
like hl7 v2 is fixed text right? because very old laboratory systems work with
that", then "this research is very very important, because we should build
this support from the start right and then maybe we should also add this
already in 0.0.4, so we are the only software that has from old to the newest
versions".

**Why:** the bridge's value is that one deployment covers every system a
hospital still runs, from a laboratory analyser's LIS speaking HL7 v2.x to an
R5 FHIR client; a design that admits only the newest wire format leaves the
oldest systems out.

**How to apply:** #248 holds the research (production evidence, specification
sources and licences, entry points); the owner decides the build order there
and the work is scheduled into the current milestone. Every design choice
from now on keeps a legacy wire format in view: a new seam in the server or
the mapping foundation must not assume FHIR R4 is the only inbound shape, and
a legacy message enters through an adapter that renders the canonical form
the existing mapping languages consume (FHIR through HL7's v2-to-FHIR pivot,
or an openEHR composition), never through a fourth hand-coded path.

**Decided 2026-09-25 (#248):** HL7 v2.x is the first legacy face, through
HL7's v2-to-FHIR corpus into the FHIRconnect path (#251). The owner ruled
"you must vendor it": the v2 definitions are vendored as a generator input
like the FHIR packages, and the HL7 licence condition on incorporating the
specification (organisational membership) is the owner's to hold, recorded in
the provenance beside the material. The ConceptMap interpreter counts as an
adapter over a published corpus, admitted under the one-core constraint.

