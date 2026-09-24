<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
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
| FHIRconnect | v1.0.0 | the only released version; its two published JSON schemas are vendored and exercised, and FerroBRIDGE adds a strict schema of its own because the published ones reject three of the specification's own mapping types |
| FHIR | R4 (4.0.1) | the only value the FHIRconnect schemas admit for `spec.version`, and what the published mapping library targets |
| OMOCL | v1.0.0 | the only released grammar (`grammar: OMOCL/v1.0.0`); the corpus is pinned by commit, because the git tag `v1.0.0` carries files that predate the grammar |
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

## The two model crates

The FHIR model is the `fhir-types` crate, generated in this repository from the
HL7 FHIR packages (R4 4.0.1, R4B 4.3.0, R5 5.0.0, R6 6.0.0-ballot5, THO 7.3.0)
and published to crates.io; its version line continues the one the sibling
project published up to 0.1.97 before the crate moved here, and the current
pin is the `fhir-types` row of `docs/VERSIONS.md`. The openEHR model comes
from six published crates, pinned together on their 0.0 minor line (0.0.67
today; the pin matrix is the authority):

| Crate | Used for |
|---|---|
| `openehr-base` | the RM foundation types, including partial dates |
| `openehr-rm` | the RM 1.1.0 model, its canonical JSON codec, the path parser |
| `openehr-its` | the OPT 1.4 codec, the Web Template builder, the composition builder, the ITS-REST data types |
| `openehr-query` | the AQL 1.1.0 parser and printer |
| `openehr-am` | the AOM2 types an ADL 2 template decodes into |
| `openehr-adl` | test only: compiles the ADL 2 fixtures from their `.adls` sources |

## Language and toolchain

The Rust toolchain, edition, resolver, and MSRV are pinned in the matrix and
adopted by the workspace when it lands. The documentation toolchain is pinned
too: `mdbook`, `mdbook-toc`, and `mdbook-mermaid` all carry an exact version in
the matrix and in the composite action that installs them, so this book renders
the same way in CI as it does on a laptop.
