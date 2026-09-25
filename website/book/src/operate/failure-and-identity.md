<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Failure and identity behaviour

A bridge carries clinical data between systems, so a silently dropped element
or a swallowed upstream error becomes a wrong record in the receiving system.
FerroBRIDGE fails loudly instead. The FHIRconnect engine chapter states its
guidance as recommendations, so each rule below is pinned as FerroBRIDGE's own
decision, and this page is the operator-facing statement of it.

<!-- toc -->

## All or nothing per unit

A transaction Bundle that cannot be mapped in full is refused with an
`OperationOutcome` naming every failing entry, and nothing is committed. A
batch Bundle answers per entry and counts its failures, because FHIR R4 defines
batch that way. There is no best-effort mode. On the OMOP side the unit is one
composition's whole record graph: a composition becomes many linked rows, and
they commit together or not at all, so no `FACT_RELATIONSHIP` row ever points
at a record that was never written.

## Type coercion is strict

A value that does not parse as the RM leaf type the Web Template resolved is a
refusal. FerroBRIDGE does not round a number, truncate a string, or guess a
date format to make a value fit.

## An upstream failure stays a failure

A refused CDR call, a failed terminology lookup, or a timeout is reported with
the upstream status. It never becomes an empty value, a default, or a silently
missing element. When a display cannot be resolved through `$lookup`, you get
an error that says so, not a resource with a coding missing its display.

## Unmapped OMOP codes are recorded, never dropped

A source code with no standard concept lands as `concept_id = 0` with the
source value kept in its source column. That is the CDM's own answer for "no
matching concept", and it keeps the row auditable. A row is never discarded
because its code did not resolve, and every run reports how many codes landed
there, per mapping, because the reference implementation's own authors measured
8.65% of primary concepts doing so and called that an underestimate.

## Identity

On the FHIR side an exported resource id is derived deterministically, as the
specification recommends, from the composition's stable `versioned_object_uid`,
the entry path and the split occurrence, so it stays the same across
composition versions, which FHIR requires of a logical id; `meta.versionId`
carries the openEHR version. FerroBRIDGE also keeps a persistent id-map: the
patient identifier to `ehr_id` relation, the external to internal resource id
relation, and the resource id to composition relation, so a `PUT` resolves and
a re-sent Bundle is recognised.

## A re-sent transaction commits once

Every entry the facade commits records the source it consumed. A resource is
keyed by its `resourceType`, `id` and `meta.versionId`, the same key a single
create uses. A message is keyed by its type, its control id and the entry's
position in the Bundle, so a redelivered message is recognised even when it is
rebuilt with fresh resource ids. When every entry of a Bundle was consumed
before, the Bundle commits nothing and answers the resources the first delivery
created. When only some were, the Bundle is refused with `409` and nothing is
committed. An entry without an `id` has no key, so its Bundle commits again
each time it is sent. These rules are FerroBRIDGE's own design.

The CDR does not say in which order it lists the versions of a contribution.
After the commit, FerroBRIDGE reads each version back and matches it to the
entry it came from by the `FEEDER_AUDIT` the engine wrote: the item's type,
its id and its version. Where several entries name the same item, as the
entries of one message do, the composition content decides. That comparison
masks what two mappings of one entry can differ in without the source
changing: the composition `uid` the CDR assigns, and every time the engine
filled from its clock because no mapping wrote it. The engine reads the clock
once per resource and hands that instant to the builder as the `ctx/time`
default, which fills `EVENT_CONTEXT.start_time`, `HISTORY.origin`,
`EVENT.time` and `ACTION.time` where the mapping left them empty (ITS-REST
Simplified Formats, master06 §time). Each `DV_DATE_TIME` whose value is that
instant is left out of the comparison, so a Bundle mapped again at a later
instant still matches the compositions its first delivery stored. A version that
matches no entry or more than one refuses the answer with a `500` that names
the contribution, and no binding is recorded.

The commit is two steps. First the facade commits the contribution with
`Prefer: return=representation` and records the contribution uid the CDR
answers against every keyed entry of the Bundle, before it binds any of them.
Then it binds each entry from the versions the answer lists. A CDR that
ignores the header answers an empty `201` that still names the contribution,
and the facade then reads the contribution back with
`GET /ehr/{ehr_id}/contribution/{contribution_uid}` and binds from its
versions, with the same `FEEDER_AUDIT` check. A contribution that cannot be
read back or bound is a `500` naming the contribution, and the recorded uid
stays. When you send the same Bundle again, the facade finds that uid,
commits nothing, reads the contribution back and binds from it, and answers
`200 OK` for each entry as for any re-sent Bundle.

A single create shares that rule. When you `POST` a resource whose `id` and
`meta.versionId` a transaction committed and did not finish binding, the
facade reads the recorded contribution back, finds the version whose
`FEEDER_AUDIT` names the resource, binds it, commits nothing, and answers
`200 OK` with the composition the first delivery produced. The other versions
of that contribution belong to the transaction's other entries and are passed
over.

Two deliveries of one source that overlap commit once. Before a delivery reads
the identity map, it claims the key of every entry it carries, and it holds
those claims until it answers, whether it commits, is refused, or fails. A
second delivery that finds any of its keys claimed commits nothing and answers
`409 Conflict` with an `OperationOutcome` whose `duplicate` issues name each
entry another delivery holds. The rule is the same for a transaction and for a
single create of a resource with an `id`. Retry after the first delivery has
answered: the retry is then recognised as a re-sent Bundle or resource, as
described above. The claims live in the server process, one set per identity
store, so they order deliveries that reach the same FerroBRIDGE instance.

On the OMOP side every CDM v5.4 primary key is a 32-bit integer, so ids come
from database sequences and a bridge-owned side table maps each source record
to its row. That table is what makes a re-run replace rather than duplicate,
and it keeps the openEHR identity in the `*_source_value` columns.

The FHIR scheme has a failure mode worth knowing before you run it: a re-sent
Bundle that omits one resource reads as a different mapping. It is documented
rather than hidden.

## Version mismatch is refused at load time

The mapping version, the grammar version, the archetype revision, the template
`sem_ver`, and the profile version all have to agree. A mismatch is refused when
the mapping loads, not when a request arrives, so a bad configuration cannot sit
waiting for the first patient record to expose it.

## Programmed mappings are compiled in

`mappingCode` in FHIRconnect and `CustomMapping` in OMOCL name a function.
FerroBRIDGE registers those as named Rust functions at build time. There is no
runtime plugin loading, so what a deployment can execute is fixed by the binary
you audited.
