<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The FHIR facade

The FHIR side of FerroBRIDGE is a REST facade in front of an openEHR CDR. A
FHIR client sees one FHIR server; behind it, every request becomes CDR
operations driven by FHIRconnect mappings. The facade stores no clinical data
of its own. It is designed and not yet built, so treat this page as the
contract being implemented rather than a description of a running server.

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
for `spec.version`. The facade answers create, update, and read, plus
transaction and batch Bundles including the conditional forms `If-None-Exist`
and `If-Match`.

A Bundle is split by context profile URL, following the specification's engine
chapter. A resource that two mappings both need becomes a linked mapping rather
than two independent conversions. An unresolved reference is fetched from the
sending site, with cycle protection.

## Paths are written as well as read

The engine is a bidirectional path model over FHIR JSON, because a mapping has
to construct a resource, not only read one. FHIRPath evaluation is used only
for the read-side expressions the mappings actually contain: `ofType()`,
`extension(url)`, and `resolve()`. FHIRconnect's `^` parent operator and
`$fhirRoot` are not FHIRPath, and the path model handles them directly.

## Terminology

The facade calls a configured FHIR terminology server for three operations:
`$lookup` to resolve a code's display, `$translate` for a `conceptmap`
reference, and `$validate-code`. The `$lookup` case is the common one, because
openEHR requires `DV_CODED_TEXT.value` while FHIR leaves `display` optional. A
terminology failure is an error, never a coding with a missing display.

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
