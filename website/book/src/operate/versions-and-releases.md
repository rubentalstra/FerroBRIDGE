<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Version lines and releases

FerroBRIDGE publishes two things, and they carry two version numbers that move
independently. Reading a release note or a crate version means knowing which
line you are looking at.

<!-- toc -->

## The product line

The product is the `ferrobridge` binary, its container image, and the
repository as a whole. Its version is the workspace `version` in the root
`Cargo.toml`, which every member inherits, and it is what a `vX.Y.Z` git tag
names. A release is cut from that tag: the lane takes its notes from the
matching `CHANGELOG.md` section, publishes the release, and the same number
appears in `CITATION.cff` and in the product row of
[`docs/VERSIONS.md`](https://github.com/rubentalstra/FerroBRIDGE/blob/main/docs/VERSIONS.md).
The milestone line is 0.0.x today, so the first product version is 0.0.1.

Where to read it:

| You want | Read |
|---|---|
| the version a binary reports | `ferrobridge --version` |
| the version a release ships | the `vX.Y.Z` release on GitHub, and its changelog section |
| the version the source tree declares | the root `Cargo.toml` `[workspace.package]` `version` |

## The crate line

The library crates are published on crates.io under plain names so other Rust
projects can depend on them: `fhir-types`, `openehr-mapping-core`,
`fhirconnect`, `omocl`, `omop-cdm`, `ferrobridge-openehr`, and
`ferrobridge-term`. Each carries the `version` in its own
`crates/*/Cargo.toml`, and that number never adopts the product version or a
specification version.

The set is deliberately not lockstep. `fhir-types` carries a real 0.1.x line,
because the crate moved here from the sibling terminology server and continues
the version sequence it already had on crates.io. Every other member still
sits at the 0.0.0 placeholder that holds its name on the registry until its
first real release. A 0.0.0 on crates.io is a name reservation and nothing
else: it compiles, it carries the licence and the metadata, and it is not the
crate you want to depend on yet.

Where to read it:

| You want | Read |
|---|---|
| the version a crate publishes | the crate's page on crates.io, or `cargo add <crate>` |
| the version the source tree declares | that member's `crates/*/Cargo.toml` `version` |
| the line `fhir-types` is on | the crate-line row of `docs/VERSIONS.md` |

## Why they are separate

A published crates.io version is immutable: the bytes under `fhir-types 0.1.2`
are the bytes under `fhir-types 0.1.2` forever. So a crate has to move its
version whenever its packaged content changes, which happens far more often
than a release is cut, and far more often for one member than for another.
Tying either number to the other would force a product release for a crate fix,
or a crate republish for a release with no library change.

Not every bumped crate version is published, so gaps in the published sequence
are normal. Publishing different content under an existing version is the one
thing that is forbidden, and crates.io refuses it.

The rule that keeps a crate version honest, and the guard that enforces it, are
on the [crate versions](../contribute/crate-versions.md) page.

## Licences

Every published member is BUSL-1.1 except `fhir-types`, which is Apache-2.0:
it is generated from the HL7 FHIR packages and exists to be usable by any Rust
project. The [licensing](../evaluate/licensing.md) page carries the terms.
