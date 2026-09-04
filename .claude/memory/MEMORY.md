<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

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
- [Licence: BUSL 1.1](license-busl.md): owner decision 2026-09-04 (#12), the
  Business Source License 1.1 on FerroEHR's and FerroTERM's terms, replacing
  the 2026-09-03 Apache 2.0 choice; non-commercial production free, commercial
  production needs a licence, Apache 2.0 four years after each version;
  inbound equals outbound, no contributor licence agreement
- [Sibling projects](sibling-projects.md): FerroEHR at `../ferroehr` is the
  reference CDR and FerroTERM at `../FerroTERM` the reference terminology
  server; both are read-only prior art from here and are never edited from
  this repository
- [Domain ferrobridge.eu](domain-ferrobridge-eu.md): the public domain is a
  Pages setting mirroring ferroterm.eu, never a `CNAME` file; owner 2026-09-04
- [Milestones 0.0.x](milestones-0-0-x.md): milestones start at v0.0.1 and step
  by a patch number, never a v0.1.0 opener; owner 2026-09-04
- [PR auto-merge](pr-auto-merge.md): enable auto-merge on every pull request
  the moment it is opened (`gh pr merge <n> --auto --squash --delete-branch`);
  owner 2026-09-04
- [Memory lives in the repo](memory-lives-in-repo.md): every learning is a
  tracked file in `.claude/memory/`, never a per-user note; owner 2026-09-04
