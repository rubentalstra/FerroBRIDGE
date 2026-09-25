---
name: hl7v2-test-corpora
description: Which public HL7 v2 message sets FerroBRIDGE vendors for the v2 face's end-to-end tests, their licences, and the fact that no real de-identified traffic is public
metadata:
  type: project
---

Research on 2026-09-25 (issue #292): every public HL7 v2 message set is
hand-written or generated test data; no real de-identified traffic is
downloadable anywhere. Real traffic needs a partner feed under an agreement.

The sets chosen: microsoft/FHIR-Converter samples (MIT, 139 messages, 132 R4
Bundles, all six families), CDCgov/prime-reportstream datatests (CC0, 383
ORU/OML/ORM with 321 Bundles, the only MSH-18 and batch variety), and the
HL7/v2-to-fhir benchmark messages (Apache-2.0, one per family). NIST
hit-mu-tools bundles (public domain, no LICENSE file) and AIRA mqe (no
licence) are fetched at build time, never vendored.

**Why:** the corpus test is the v2 face's conformance instrument; the expected
Bundles are a comparison oracle for the interpreter, not ground truth, since
they reflect the converters' own template choices.

**How to apply:** pins live in `docs/VERSIONS.md`, the vendor script is
`scripts/vendor/hl7v2-samples.sh`, the pass list is `conformance/hl7v2/`.
Z-segments and odd MSH-2 delimiters stay in-tree derived fixtures
([[legacy-standards-first-class]], [[vendored-inputs]]).
