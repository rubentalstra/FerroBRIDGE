<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# ferrobridge-server

The `ferrobridge` binary: one process, one subcommand per job.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.
This crate is never published to crates.io.

## The subcommands

| Command | What it does |
|---|---|
| `ferrobridge serve` | Serves the HTTP surface until the process is asked to stop |
| `ferrobridge etl run` | Loads compositions from the CDR into the CDM database |
| `ferrobridge cdm init` | Applies the OMOP CDM v5.4 DDL |
| `ferrobridge vocab load` | Loads an OHDSI vocabulary export |
| `ferrobridge mapping check` | Validates the configured mapping files |

`serve` runs today. Every other job parses, then exits with code 2 and one
line on stderr naming the tracker issue that lands it.

`--config <PATH>` names the configuration file, and `--version` prints the
product version.

## Configuration

An optional TOML file, then environment variables over it. The file is what
`--config` names, or what `FERROBRIDGE_CONFIG` names; without either, the
defaults stand. An override is `FERROBRIDGE__<SECTION>__<KEY>`, two
underscores per level, so `FERROBRIDGE__CDR__RETRY__MAX_ATTEMPTS` sets
`[cdr.retry] max_attempts`.

Every struct refuses an unknown key, and a bad value is refused with the key
named. Every credential is reachable through a `<key>_file` sibling read once
at boot; setting a value and its `_file` together is a boot error. A refused
configuration exits 78, `EX_CONFIG`.

A lane is off until its section is present: no `[cdr]` means no CDR client and
no CDR readiness indicator. The variable table lives on the Operate page of the
book.

## The HTTP surface

| Route | Answer |
|---|---|
| `GET /` | A JSON document naming the product and the version |
| `GET /health/liveness` | `200` while the process is up |
| `GET /health/readiness` | `200` when every indicator is up, `503` with each indicator's state otherwise |

Readiness runs one indicator per configured upstream, on the request. A probe
that reaches the upstream counts it up, `401` and `404` included; a `5xx` and a
failure to connect count it down. With the facade on, one more indicator reads
the identity store.

The middleware stack, outermost first: the request-id normalizer, the layer
that mints one, the panic renderer, the layer that propagates the id, the panic
catcher, the request timeout (`408`), the body ceiling (`413`), and the request
log.

## The FHIR facade

`[facade] enabled = true` mounts the FHIR R4 surface under `/fhir`, over the
CDR client and the mapping set `[mappings] directory` names. Off, it mounts no
route, so a request answers `404` rather than `403`.

| Route | Answer |
|---|---|
| `GET /fhir/metadata` | The `CapabilityStatement` of the loaded programs |
| `POST /fhir/{type}` | Create, conditional through `If-None-Exist` |
| `POST /fhir/{type}/$validate` | The dry run that commits nothing |
| `GET /fhir/{type}/{id}` | Read |
| `PUT /fhir/{type}/{id}` | Update, with `If-Match` |
| `POST /fhir` | A `transaction` Bundle, committed as one contribution |

The facade stores no clinical data. What it keeps is identity, in a `redb` file
the lane opens at boot: a subject to its `ehr_id`, a resource id to the
composition entry it came from, and the source identity of every resource it
consumed, which is what makes a re-sent resource update one composition instead
of creating a second. A CDR answer becomes a FHIR answer through one status
table, with both sides cited per row. The book's Integrate page carries the
surface in full.

## What reaches the log

One line per request, with the method, the matched route, the status, the
latency, and the request id. Never a body, never the raw URL, and a query value
only when the deployment named its parameter in
`[telemetry] logged_query_parameters`, cut at 64 characters.

An `X-Request-Id` a client sends is echoed when it is printable ASCII of at
most 128 characters; anything else is replaced by a minted version 4 UUID, so a
header cannot forge a log line.

## Stopping

`SIGTERM` and `SIGINT` start a drain bounded by `[server] shutdown_timeout_ms`.
A request in flight finishes; a connection still open when the drain elapses is
dropped.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
