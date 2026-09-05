<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Pinned versions

Every version FerroBRIDGE targets is pinned in one place,
[`docs/VERSIONS.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/docs/VERSIONS.md),
and a committed guard (`scripts/checks/versions.sh`) fails when any file that
repeats a pin disagrees with it. This page is the reader's copy of the
specification pins and the reason each one is what it is.

## The specification pins

| Component | Pin | Why |
|---|---|---|
| FHIRconnect | v1.0.0 | the only released version; its two published JSON schemas are the validation oracle |
| FHIR | R4 (4.0.1) | the only value the FHIRconnect schemas admit for `spec.version`, and what the published mapping library targets |
| OMOCL | v1.0.0 | the only released grammar (`grammar: OMOCL/v1.0.0`) |
| OMOP CDM | v5.4 | the only version the published OMOCL files declare |
| openEHR ITS-REST | 1.1.0 | the released REST API a conformant CDR speaks |

The FHIRconnect prose says other FHIR releases should work, and nothing tests
that claim, so FerroBRIDGE does not make it. A later FHIR release is
feature-gated in the model crate and claimed once a mapping corpus proves it.

## Terminology operations

The FHIR terminology client speaks `CodeSystem/$lookup`,
`ConceptMap/$translate`, and `ValueSet/$validate-code`, and tolerates a server
answering R4 or R4B. The terminology server is configured, never assumed. OMOP
concept resolution does not go through it at all; it is SQL over the loaded
OHDSI vocabulary.

## The FHIR model comes from crates

FerroBRIDGE generates no FHIR code. It depends on two published crates, pinned
by version like any other dependency:

| Crate | Version | Used for |
|---|---|---|
| `fhir-types` | 0.1.22 | the R4 resource and data types the facade and the terminology client speak |
| `fhir-terminology` | 0.1.22 | the request and response contracts of the terminology operations |

## Language and toolchain

The Rust toolchain, edition, resolver, and MSRV are pinned in the matrix and
adopted by the workspace when it lands. The documentation toolchain is pinned
too: `mdbook`, `mdbook-toc`, and `mdbook-mermaid` all carry an exact version in
the matrix and in the composite action that installs them, so this book renders
the same way in CI as it does on a laptop.
