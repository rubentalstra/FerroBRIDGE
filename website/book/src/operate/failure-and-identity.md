<!-- SPDX-FileCopyrightText: Ruben Talstra -->
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
