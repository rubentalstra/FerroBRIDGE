<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The FHIR facade

The FHIR side of FerroBRIDGE is a REST facade in front of an openEHR CDR. A
FHIR client sees one FHIR server; behind it, every request becomes CDR
operations driven by FHIRconnect mappings. The facade stores no clinical data
of its own.

The facade runs. It is off until `[facade] enabled` turns it on, and a disabled
facade mounts no route, so a request answers `404` rather than `403`. What this
page describes is what the server answers today: conformance, create, read,
update, transaction and `$validate`. Search and batch are later work, and the
`CapabilityStatement` leaves them out rather than claiming them.

<!-- toc -->

## Why a facade

FHIRconnect defines the mapping language and leaves the server surface to the
implementer. Two shapes exist in the prior art: publish a facade, or leave both
the facade and the CDR to the integrator. No standard prefers either.
FerroBRIDGE publishes the facade so that a FHIR client needs to know about one
server and one base URL, and so that the mapping direction, the terminology
calls, and the CDR commit happen in one place that can refuse a request as a
whole.

## The surface

FHIR R4 (4.0.1), because that is the only version the FHIRconnect schemas admit
for `spec.version`. The service base is `/fhir`.

| Request | What it does |
|---|---|
| `GET /fhir/metadata` | The `CapabilityStatement` of the loaded mapping set |
| `POST /fhir/{type}` | Create, conditional through `If-None-Exist` |
| `POST /fhir/{type}/$validate` | The dry run that commits nothing |
| `GET /fhir/{type}/{id}` | Read one resource back out of its composition |
| `PUT /fhir/{type}/{id}` | Update, with `If-Match` |
| `POST /fhir` | A `transaction` Bundle, all or nothing |

A type no loaded program maps is absent from the `CapabilityStatement` and
answers `404` with a `not-supported` `OperationOutcome` on the wire. A batch
Bundle answers `422` with `not-supported`. Search has no route
([issue #98](https://github.com/rubentalstra/FerroBRIDGE/issues/98) lands it),
and a `_count` that is not a non-negative integer is a `400` with `invalid`,
so a client never reads a page it did not ask for.

The `CapabilityStatement` names exactly the resource types the loaded programs
map, with `create`, `read` and `update` per type, `transaction` at system
level, the `$validate` operation, `updateCreate: false`, `conditionalCreate`
and `conditionalUpdate` true, `fhirVersion: 4.0.1`, and no search parameter.
When the FHIRconnect operations lane is served under the same base,
`rest.operation` also names `$tofhir` and `$toopenehr`; their direct forms are
never declared ([The FHIRconnect operations](operations.md) says why).

## Media types and bodies

The facade reads `application/fhir+json` and `application/json`, which R4 names
as the JSON media type and its alias, and answers `415` for anything else. Every
response carries `application/fhir+json`. An `Accept` header that admits neither
is a `406`.

A body is parsed through the strict codec of the generated `fhir-types` crate,
so an unknown property is a `400` with `structure` rather than a value that is
silently dropped. `Prefer: return=minimal`, `return=representation` and
`return=OperationOutcome` are honoured on a write, and the default is the
resource itself.

Everything the facade authors is an `OperationOutcome` with an `issue.code`
from the R4 value set. An openEHR error body never reaches the wire as its own
document: the CDR's `message` and every `validationErrors` entry travel
verbatim inside `issue.diagnostics`, which keeps one error vocabulary on the
wire while losing nothing the CDR said.

## Program selection

A request is mapped by the program whose context claims the profiles the
resource declares in `meta.profile`. Every declared profile is considered, as
set membership, and the `templateId` pin of the FHIRconnect operations surface
selects among candidates when a deployment sends one.

Two refusals are distinct on the wire, and both are `422`. When no program
claims the resource, the outcome names the profiles it saw and answers
`not-supported`. When several programs claim it and nothing pins the template,
the outcome names the candidates, so the fix is a pin rather than a guess.

## Identity

A FHIR resource id has to be stable across composition versions, because R4
fixes that "once assigned, this value never changes". The facade derives it
from the entry's `LOCATABLE.uid` when the entry carries one that the R4 `id`
grammar admits, and otherwise from a SHA-256 over the version container, the
entry path and the split occurrence, rendered as 52 lowercase base32
characters. `meta.versionId` carries the openEHR `version_tree_id` and never
enters the derivation, so a new version of a composition reads back under the
same id with a new `meta.versionId`. `meta.source` names the openEHR version
uid the answer was read from.

The identity map is a `redb` file with four tables: a patient identifier to its
`ehr_id`, an external resource id to the internal one, an internal id to the
composition version container with the entry path and split occurrence, and the
source resource `id` with its `meta.versionId` to the mapping that consumed
them. The map wins once written, so a derivation change never renames a
resource a client already holds. The store keeps identifiers and nothing else:
no clinical content is written into it, and a test greps the file for a mapped
value to prove it.

That last table is what makes create idempotent. A resource re-sent with the
same `id` and `meta.versionId` updates the composition it already produced
instead of creating a second one.

## Provenance

Every composition the facade commits carries a `FEEDER_AUDIT`, which is the
reference model's own element for data transformed into openEHR form: the
source resource's `id` and type as an `originating_system_item_ids` entry, its
`meta.versionId` as `originating_system_audit.version_id`, and the configured
`system_id` naming the bridge. A source resource that carries no `id` is
recorded as unknown, never given one. So a reader of the CDR can tell which
FHIR resource a composition came from without asking the bridge.

## Writes

A create resolves the subject to an EHR: the identity map first, then the CDR
by subject id and namespace, and only then the configured policy. With
`ehr_policy = "existing"` an unknown subject is refused; with
`create_on_first_write` the facade creates the EHR and records it.

`If-None-Exist` behaves as R4 defines it: no match creates, one match answers
`200` with the resource that already exists, and several matches answer `412`.
The search it runs is over the identity map, so the parameters it answers are
`_id` and `identifier`, each in the comma-separated OR form. Any other
parameter is refused, because a silently narrowed search would turn a duplicate
into a second composition.

A create answers `201` with `Location` naming the FHIR resource under the
configured base URL and `ETag` carrying the version.

An update needs `If-Match` when the map knows the id. Without one, the facade
reads the CDR's current `ETag` and answers `412` with it on a mismatch rather
than overwriting a version the client never saw. A `PUT` to an id the map does
not know is a `404`: this milestone does not upsert, and the
`CapabilityStatement` says `updateCreate: false`.

A `transaction` Bundle maps every entry first, resolving references within the
Bundle by `fullUrl` and then through the sending site with cycle protection,
and commits the lot through one ITS-REST contribution so the CDR makes it
atomic. Any failure answers one `OperationOutcome` naming every failing entry
by `fullUrl`, and nothing is committed.

A committed transaction answers `200` with a `transaction-response` Bundle,
one entry per request entry in the same order. Each entry's `response` carries
`201 Created`, the `location` `[base]/[type]/[id]/_history/[vid]` and the
`etag` `W/"[vid]"`, the values a single create answers. Send the same Bundle
again and nothing is committed: each entry answers `200 OK` with the location
of the resource the first delivery created. A Bundle only some of whose
entries an earlier delivery consumed is refused with `409`, naming those
entries, and a Bundle that carries one resource twice is refused with `422`.
A delivery that arrives while another delivery of the same Bundle or resource
is still in flight is refused with `409` and commits nothing; retry it after
the first one answers. The redelivery rule is on the
[failure and identity](../operate/failure-and-identity.md) page.

## `$validate` is a dry run of this server

`POST /fhir/{type}/$validate` runs the whole inbound path up to the commit and
commits nothing, which a test proves by counting EHRs and versions before and
after. It answers `200` for both verdicts: `information` issues when the
resource would commit, and an `error` issue carrying the validator's message
verbatim when it would not. The operation-level statuses mirror the create
path, so a `415`, a `422` for an unmapped profile or a `404` for an unknown
type reads the same on both routes.

The validator is this server, not the CDR. openEHR ITS-REST 1.1.0 documents no
composition-validation route, and a commit the bridge never finalises would
still write a version, so what runs is the mapping engine, the Web Template
validation the composition build performs, and the strict RM reader. Every
outcome says so in `issue.details`, so you can read what a verdict covers. A
CDR that refuses a composition this validator accepts still answers `422` on
the create.

## Status mapping

No specification governs what a CDR answer becomes on the FHIR wire, so the
mapping is FerroBRIDGE's own design. It lives in one table, each row citing
both sides, and one wire test asserts each row.

| The CDR answers | The facade answers |
|---|---|
| `422`, template validation | `422` with `validationErrors` verbatim in `issue.diagnostics` |
| `412` | `412` carrying the current `ETag` |
| `204` on a read, the composition is deleted | `410` |
| `404` | `404` |
| `401` | `401`, propagating `WWW-Authenticate`, never `403` |
| `400`, `405` or `415` | `500`: the bridge chose a call the CDR does not offer |
| any `5xx` | `502` carrying the upstream status |
| no answer: a connect failure or a timeout | `502` |

## Paths are written as well as read

The engine is a bidirectional path model over FHIR JSON, because a mapping has
to construct a resource as well as read one. The model is guided by the element
table of the generated `fhir-types` crate, so it knows which elements repeat,
which are choice types, and which alternatives a choice admits. The three
read-side FHIRPath forms the published mappings use (`ofType()` and `as()`,
`extension(url)`, `resolve()`) are path-model operations; no FHIRPath evaluator
is embedded. FHIRconnect's `^` parent operator and `$fhirRoot` are not FHIRPath,
and the path model resolves them when the mapping is compiled.

## Terminology

The facade calls a configured FHIR terminology server for three operations:
`$lookup` to resolve a code's display, `$translate` for a `conceptmap`
reference, and `$validate-code`. The `$lookup` case is the common one, because
openEHR requires `DV_CODED_TEXT.value` while FHIR leaves `display` optional. A
terminology failure is an error, never a coding with a missing display.

The facade does not call the terminology server yet. The call site is the
inbound engine seam, and it runs the calls before anything is built, so a
terminology refusal will cost no CDR write. It arrives with the registry of
mapping functions in
[issue #95](https://github.com/rubentalstra/FerroBRIDGE/issues/95); until then
a mapping that needs a display resolved refuses the unit rather than writing a
coding without one.

## Search is FerroBRIDGE's own design

FHIRconnect states that it "does not focus specifically on AQL and FHIRsearch",
so no specification governs FHIR search here. The design is FerroBRIDGE's own,
and it is labelled as such wherever it appears:

- An AQL projection per resource type, declared beside the context mapping.
- Execution over `POST /query/aql` on the CDR.
- A CapabilityStatement that names exactly which search parameters each
  resource supports.
- An `OperationOutcome` refusing every parameter outside that list, rather than
  ignoring it.

A refusal is deliberate. A search server that silently drops an unsupported
parameter returns a result set that looks filtered and is not, which is the
kind of quiet wrongness this project treats as a defect.

## What the facade does not do

It does not materialise a FHIR store, it does not implement Subscriptions, and
it does not send demographics to an external endpoint. Each of those is
recorded as outside the current design, with its reasoning, on the tracker.
