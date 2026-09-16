<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# ferrobridge-term

The FerroBRIDGE FHIR terminology client for `$lookup`, `$translate` and
`$validate-code`.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

## What it does

Three operations on one configured server:

- `CodeSystem/$lookup` resolves a code's display.
- `ConceptMap/$translate` maps a code into another code system.
- `ValueSet/$validate-code` tests membership of a value set.

`batch` sends several of them as one `batch` `Bundle` and answers them by
position; an entry the server refused is that entry's own typed error and
leaves the others alone.

A negative answer an operation states in its own `out` parameters is an
outcome: `LookupOutcome::NotFound`, `TranslateOutcome::NoMatch`,
`ValidateOutcome::Invalid`. Everything else is a typed error carrying the
upstream status, the body, and any `tx-issue-type` coding the
`OperationOutcome` holds. A missing display is `NotFound`, never an empty
string, so the caller decides between a refusal and a configured fallback.

`translate` returns every match with its equivalence and offers `accepted()`,
which yields the `equivalent` and `equal` matches alone. The `$translate`
definition asks the caller to check `match.equivalence` for each match, so a
`wider`, `narrower` or `inexact` answer never reaches a target system as a
translation.

The request and response contracts come from the generated
[`fhir-types`](https://docs.rs/fhir-types) operation modules; this crate models
no FHIR of its own.

## Two wire versions

`WireVersion` selects R4 or R4B. Both releases declare the same `in` and `out`
parameters for the three operations, so the release decides which generated
model reads the answer. A server that serves several releases side by side puts
the release in its base path, so the base URL and the wire version are
configured together.

## An absent server

`Client::new` is the only way to get a client, so a deployment that configures
no terminology server holds `Option<Client>` and the absence is visible at
every call site. Keep it that way: a `None` is a refusal to translate, not a
reason to pass a source code through under a target system.

The `Client::translate` documentation carries the worked example.

## Status

The crate version is a placeholder while the crate line settles; the tracker
carries the build order.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
