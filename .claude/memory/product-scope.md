---
name: product-scope
description: "What FerroBRIDGE is (the owner's product statement), the standalone-server decision, research-first on issue #1, and the expectation that FerroEHR's in-tree FHIR extension retires in favour of the bridge"
metadata:
  node_type: memory
  type: project
---

<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

The product statement, as the owner gave it, is the ceiling on what this
repository may claim:

> FerroBRIDGE is a pure-Rust, standalone bridge between openEHR and two
> interoperability targets: HL7 FHIR, driven by the FHIRconnect specification
> (model-mapping and contextual-mapping YAML validated against its published
> JSON schemas), and the OMOP Common Data Model, driven by the OMOCL
> specification. Mappings are specification-conformant YAML, never hand-coded
> per resource or table. It runs as its own server beside any openEHR CDR
> reached over the openEHR ITS-REST API (FerroEHR is the reference CDR, never a
> compile-time dependency) and uses a FHIR terminology server (FerroTERM is the
> reference) for code systems and value sets.

**Standalone server, decided.** FerroBRIDGE is its own process and its own
image. It is not a crate inside FerroEHR, not a plugin, and not a library a CDR
links. It reaches the CDR over ITS-REST like any other client, which is what
lets it work against a CDR it did not build.

**Mapping-driven, decided.** Mappings are specification-conformant YAML
validated against published schemas. A hand-coded per-resource or per-table
mapping defeats the premise, so it is refused in review
(`.claude/rules/rust-style.md`).

**Research first, on issue #1.** Everything else is open: the crate layout, the
engine design, whether a generated model layer exists on either side, the
version pins, and the acceptance instrument. `docs/architecture.md` does not
exist and is the output of that program. Do not scaffold ahead of it
(`owner-work-style.md`), and make no technical claim about a target beyond the
statement above or a cited research finding.

**FerroEHR's in-tree FHIR extension is expected to retire.** FerroEHR ships
FHIR conversion inside `app/ferroehr-ext` behind a cargo feature. Once
FerroBRIDGE exists, that extension is expected to be removed in favour of the
bridge, so the CDR stops carrying a second FHIR implementation. That retirement
is FerroEHR's own tracker work, decided and executed there, never from this
repository (`sibling-projects.md`). Design here as if the CDR has no FHIR
surface of its own.
