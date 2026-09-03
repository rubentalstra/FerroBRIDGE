<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Memory index

- [Product scope](product-scope.md): the owner's product statement is the
  ceiling on what this repository may claim; standalone server and
  mapping-driven are decided, everything else is research on issue #1;
  FerroEHR's in-tree FHIR extension is expected to retire in favour of the
  bridge
- [OMOP target](omop-target.md): the owner confirmed OMOP as a first-class
  target beside FHIR on 2026-09-03; FHIRconnect covers FHIR (openFHIR is its
  reference implementation) and OMOCL covers OMOP (Eos is its reference
  implementation); origin FerroEHR #2652, now FerroBRIDGE #2; claim nothing
  about OMOP beyond the product statement
- [Owner work style](owner-work-style.md): research-first and evidence-based,
  from first principles; confirm foundational decisions before scaffolding; no
  code while the design is open
- [Licence: Apache 2.0](license-apache.md): owner decision 2026-09-03,
  FerroBRIDGE stays Apache 2.0 while FerroEHR and FerroTERM moved to BUSL 1.1
  the same day, because the bridge is meant to be adopted widely; inbound
  equals outbound, no contributor licence agreement
- [Sibling projects](sibling-projects.md): FerroEHR at `../ferroehr` is the
  reference CDR and FerroTERM at `../FerroTERM` the reference terminology
  server; both are read-only prior art from here and are never edited from
  this repository
