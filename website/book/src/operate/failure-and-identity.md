<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Failure and identity behaviour

A bridge carries clinical data between systems, so a silently dropped element
or a swallowed upstream error becomes a wrong record in the receiving system.
FerroBRIDGE fails loudly instead. The FHIRconnect engine chapter states its
guidance as recommendations, so each rule below is pinned as FerroBRIDGE's own
decision, and this page is the operator-facing statement of it.

<!-- toc -->

## All or nothing per request

A Bundle that cannot be mapped in full is refused with an `OperationOutcome`
naming every failing entry, and nothing is committed. There is no partial
commit and no best-effort mode. The same rule holds for the OMOP side: a job
that cannot map a composition reports it rather than writing a half-row.

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
because its code did not resolve.

## Identity

FerroBRIDGE keeps a persistent id-map: the patient identifier to `ehr_id`
relation, and the external to internal resource id relation. An exported
resource id is derived deterministically, as a hash of the composition UID plus
the entry path, which is what the specification recommends.

That scheme has a failure mode worth knowing before you run it: a re-sent
Bundle that omits one resource hashes differently and reads as a different
mapping. It is documented rather than hidden, and it is the reason the id-map
is persistent rather than recomputed.

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
