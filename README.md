<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# FerroBRIDGE

<!-- badges:begin -->
[![CI](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/ci.yml/badge.svg)](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/ci.yml)
[![CodeQL](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/codeql.yml/badge.svg)](https://github.com/rubentalstra/FerroBRIDGE/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/rubentalstra/FerroBRIDGE/badge)](https://scorecard.dev/viewer/?uri=github.com/rubentalstra/FerroBRIDGE)
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=rubentalstra_FerroBRIDGE&metric=alert_status)](https://sonarcloud.io/summary/overall?id=rubentalstra_FerroBRIDGE)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=rubentalstra_FerroBRIDGE&metric=coverage)](https://sonarcloud.io/summary/new_code?id=rubentalstra_FerroBRIDGE)
[![License: BUSL-1.1](https://img.shields.io/badge/License-BUSL--1.1-blue.svg)](LICENSE)
[![GitHub release (latest SemVer)](https://img.shields.io/github/v/release/rubentalstra/FerroBRIDGE?sort=semver)](https://github.com/rubentalstra/FerroBRIDGE/releases/latest)
<!-- badges:end -->

A pure-Rust, standalone bridge between openEHR and two interoperability
targets: HL7 FHIR, driven by the FHIRconnect specification (model-mapping and
contextual-mapping YAML validated against its published JSON schemas), and the
OMOP Common Data Model, driven by the OMOCL specification. Mappings are
specification-conformant YAML, never hand-coded per resource or table. It runs
as its own server beside any openEHR CDR reached over the openEHR ITS-REST API,
never as a compile-time dependency of one, and uses any FHIR terminology server
for code systems and value sets.

## Status: designed, not yet built

The research pass over the specifications and the prior art is done and its
result is [`docs/architecture.md`](docs/architecture.md): FHIRconnect and OMOCL
are two mapping languages sharing one header, so FerroBRIDGE is one shared
foundation with two interpreters and two sinks; it pins FHIRconnect v1.0.0 on
FHIR R4 and OMOCL v1.0.0 on OMOP CDM v5.4; the FHIR side is a REST facade over
the CDR and the OMOP side a batch ETL into a CDM database. The first milestone is
one verbatim round trip per target on upstream mapping files. Follow
[issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1) for the
decisions and their citations.

## Documentation

The site is <https://ferrobridge.eu/>, and the book lives under
<https://ferrobridge.eu/docs/>. It is organised by what you came to do:

- [Architecture](https://ferrobridge.eu/docs/evaluate/architecture.html) and
  [the two mapping specifications](https://ferrobridge.eu/docs/evaluate/mapping-specifications.html),
  to judge whether FerroBRIDGE fits your problem.
- [What FerroBRIDGE runs beside](https://ferrobridge.eu/docs/operate/deployment-shape.html)
  and [failure and identity behaviour](https://ferrobridge.eu/docs/operate/failure-and-identity.html),
  to plan a deployment.
- [The FHIR facade](https://ferrobridge.eu/docs/integrate/fhir-facade.html),
  [the OMOP ETL](https://ferrobridge.eu/docs/integrate/omop-etl.html), and
  [writing and loading mappings](https://ferrobridge.eu/docs/integrate/mappings.html),
  to integrate with it.
- [How the work is organised](https://ferrobridge.eu/docs/contribute/how-the-work-is-organised.html)
  and [checks and gates](https://ferrobridge.eu/docs/contribute/checks-and-gates.html),
  to contribute.

## What is decided

- **A standalone server.** FerroBRIDGE is its own process and its own image. It
  is not a crate inside a CDR, not a plugin, and not a library a CDR links. It
  reaches the CDR over ITS-REST like any other client, which is what lets it
  work against a CDR it did not build.
- **Mapping-driven, never hand-coded.** Mappings are specification-conformant
  YAML validated against the published schemas. A hand-written per-resource or
  per-table converter is refused in review, because it is the thing this
  project exists to avoid.
- **Pure Rust.** No JVM, and a single binary, like the rest of the family.
- **Business Source License 1.1.** Free for non-commercial use, a commercial
  licence for production use in a business. See below.

## What is open

The crate layout, the engine design, whether a generated model layer exists on
the FHIR side or the openEHR side, the version pins for every specification,
and the acceptance instrument. Each is a question for issue #1 rather than an
assumption, and the answer arrives with its citation.

## Licensing

FerroBRIDGE is source-available under the Business Source License 1.1
([`LICENSE`](LICENSE), [`NOTICE`](NOTICE)), with no open-core tier: the
bridge, the server, and the tools are in this repository under the one
licence, and nothing is held back to be sold back to you.

The licence lets you read, build, modify, and redistribute the source without
a fee and without asking anyone, and it covers every non-production use:
development, testing, evaluation, and prototyping. Production use is free for
Non-Commercial Purposes, which the licence defines as personal use, academic or
scientific research, teaching, and use by a non-profit organisation or public
body that is not in the course of a business, does not deliver a service for
payment, and is not for commercial advantage. Any other production use needs a
commercial licence from the Licensor: a hospital, clinic, or care provider
running FerroBRIDGE between its systems needs one, and so does a vendor,
integrator, or any company running it in production. Offering FerroBRIDGE, or a
work derived from it, to third parties as a hosted, managed, or embedded
service that maps or exchanges health data, and selling, sublicensing, or
otherwise distributing it for a fee on its own or inside another product, need
a commercial licence in every case. Each version becomes Apache License 2.0
four years after that version is published. The commercial licence starts with
a short conversation with the maintainer named in
[MAINTAINERS.md](MAINTAINERS.md).

Contributions are licensed inbound equals outbound under the same licence, and
there is no contributor licence agreement and no copyright assignment. Vendored
specifications and third-party material keep their upstream terms, recorded
beside them.

## Contributing

[`CONTRIBUTING.md`](CONTRIBUTING.md) has the rules, and the open issues are the
worklist. While the project is in its design phase the most useful contribution
is evidence: a specification citation, a measurement, or first-hand experience
with FHIRconnect, OMOCL, or an openEHR CDR in production.

- [`CLAUDE.md`](CLAUDE.md): the working discipline, in full.
- [`AI_STATEMENT.md`](AI_STATEMENT.md): how AI tools are used to build this,
  and what they are not allowed to do.
- [`GOVERNANCE.md`](GOVERNANCE.md), [`MAINTAINERS.md`](MAINTAINERS.md): who
  decides, and the honest answer about the bus factor.
- [`SECURITY.md`](SECURITY.md): report a vulnerability privately.
