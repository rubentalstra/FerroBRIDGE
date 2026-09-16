<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# The FHIRconnect operations

FerroBRIDGE serves the two operations the FHIRconnect specification defines for
its engine: `$tofhir` turns an openEHR composition into FHIR resources, and
`$toopenehr` turns FHIR resources back into a composition. Both are pure
transformations. Neither reaches a CDR, and neither changes server state, so
you can call them from a facade, a test harness or a shell.

<!-- toc -->

## The chapter is a draft

The REST API chapter is pull request #93 of the FHIRconnect specification and
is not part of any release. FerroBRIDGE implements it at the pinned commit
recorded in `docs/VERSIONS.md`, vendors the chapter and its FSH operation
definitions, and labels the wire tests draft. When the chapter merges, the pin
moves and every difference is re-adjudicated.

## Turning the lane on

Configure a mapping set. `[mappings] directory` names the tree of FHIRconnect
mapping files and `[mappings] templates` the directory of operational
templates they compile against. Both are read once at boot, so a mapping that
does not compile stops the server rather than the first request that touches
it. `[operations]` carries the lane switch and the values a run needs beside
the payload. See [Configuring the server](../operate/configuration.md).

Without `[mappings]`, both operations answer `503` with an `OperationOutcome`.

## `POST /fhir/$tofhir`

The body is a FHIR `Parameters` resource in `application/fhir+json`:

```json
{
  "resourceType": "Parameters",
  "parameter": [
    { "name": "composition", "valueString": "<the composition as a JSON string>" },
    { "name": "templateId", "valueString": "Blood Pressure" },
    {
      "name": "context",
      "part": [
        { "name": "ehr_id", "valueString": "53d89df2-5501-4455-9a65-565a5d1ddb7c" },
        { "name": "patient", "valueReference": { "reference": "Patient/123" } },
        { "name": "who", "valueReference": { "reference": "Practitioner/456" } },
        { "name": "onBehalfOf", "valueReference": { "reference": "Organization/charite" } }
      ]
    }
  ]
}
```

The composition travels as a JSON string in either serialization. A canonical
composition carries its template at `archetype_details.template_id`; a FLAT one
carries none, so `templateId` is required with it and a FLAT payload without
one answers `400` with the issue code `required`. When the payload and
`templateId` both name a template and they disagree, the call is refused.

The answer is a `collection` Bundle carrying the mapped resources, exactly one
`Provenance`, and an `OperationOutcome` entry when the run declared a loss.

## `POST /fhir/$toopenehr`

The body is a FHIR `Bundle`, or a `Parameters` carrying it under `bundle`. The
answer is a `Parameters` with the composition as a JSON string and an optional
`outcome`. `format` selects the serialization: `canonical` by default, or
`flat`.

FerroBRIDGE picks the mapping by the profiles the Bundle's resources claim in
`meta.profile`, narrowed by `templateId` when you supply one. A Bundle that
references more than one subject, or that carries more than one resource of the
mapped type, is refused: one Bundle maps to one composition.

## Query parameters

`templateId`, `format` and `ehr_id` may travel in the query instead of the
body. The two forms are equivalent, and the body wins when both carry the same
field.

```http
POST /fhir/$tofhir?templateId=KDS_Fall_einfach&ehr_id=53d89df2-5501-4455-9a65-565a5d1ddb7c
```

A structured value, a full `Reference` such as `who` or `onBehalfOf`, belongs
in the body.

## The direct form

When you have a composition and want a Bundle back, the wrapper buys you
nothing. The chapter therefore allows an un-enveloped form, and marks it as a
deliberate deviation from the FHIR operations framework that stays outside the
FHIRconnect FHIR implementation guide:

```http
POST /fhir/tofhir
Content-Type: application/openehr+json

{ "_type": "COMPOSITION", ... }
```

```http
POST /fhir/toopenehr?format=flat
Content-Type: application/fhir+json

<a FHIR Bundle>
```

`POST /fhir/tofhir` answers the Bundle itself and `POST /fhir/toopenehr`
answers the composition itself, with `Content-Type:
application/openehr+json`. The media type is the one the chapter defines for
openEHR canonical JSON, with the suffix ordering of RFC 6839 §4.

## What a failure looks like

FerroBRIDGE is strict, which the chapter leaves open. A mapping that cannot be
performed answers an `OperationOutcome` and no Bundle and no composition, so
you can never mistake a partial result for a complete one. A run that succeeded
but lost something reports each loss as an issue beside the result:
`information` for a composition field the engine filled, `warning` for content
that did not reach the output.

Every error is an `OperationOutcome` in `application/fhir+json`, with the R4
issue code that fits: `required` for a missing parameter, `structure` for a
parameter the operation does not declare or one given twice, `value` for the
wrong value type, `not-found` for a template or profile nothing answers,
`multiple-matches` when several mappings answer and nothing pins the choice,
`business-rule` for a Bundle with more than one subject, and `processing` for a
mapping that did not run to completion. A body in any other media type answers
`415`.

## Provenance on every run

Every `$tofhir` Bundle carries one `Provenance` describing the transformation:
`target` references every mapped resource, `recorded` is the run time,
`agent.who` is `context.who` or the configured `Device` reference,
`agent.onBehalfOf` is `context.onBehalfOf`, and one `entity` with
`role = derivation` names the composition the run read, by its `uid` when it
carried one.

## Resolving the patient

An openEHR composition identifies the patient through the EHR it belongs to, so
mapping to FHIR means resolving an EHR identifier to a patient. That belongs to
whatever owns patient identity in a deployment. `context.patient` is the
caller's fallback: FerroBRIDGE never requires it, and when you supply it, it
takes precedence over the subject a mapping resolved on its own. Correctness
then rests with you, because a wrong reference attaches clinical data to the
wrong patient and nothing downstream catches it.
