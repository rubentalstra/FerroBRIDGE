<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# FerroBRIDGE's strict FHIRconnect schemas

Two JSON Schema draft-07 documents, authored here from the FHIRconnect v1.0.0
specification prose and its two published schemas. They are first-party
FerroBRIDGE files under the repository's licence, not vendored material: the
published schemas stay where `scripts/vendor/fhirconnect.sh` put them, under
`docs/specs/fhirconnect/`, and nothing in this directory is a copy of them.

`model-mapping.schema.json` covers a `type: model` and a `type: extension`
file; `contextual-mapping.schema.json` covers a `type: context` file. The
loader runs these; `fhirconnect::model::schema::published` runs the published
pair over caller-supplied bytes, which is how the corpus test pins where the
published schemas and the prose disagree.

## What these add over the published schemas

Each row names the published-schema behaviour, the prose that contradicts it,
and what these schemas do instead.

| Item | Published schema | Specification prose | Here |
|---|---|---|---|
| `mappingCode` | absent from `#/$defs/mapping`, which is closed, so a PROGRAMMED mapping cannot appear | `types-of-mappings/concept-type/concept-mappings.adoc`, §Programmed mappings | a non-empty string |
| `link` | absent the same way, so a LINKED mapping cannot appear | the same page, §Linked mappings | an object with `meaning` and `type`, or null |
| `participationsFunction` | absent the same way, so a PARTICIPATION mapping cannot appear | the same page, §Participation mappings | a non-empty string |
| mapping-level `conceptmap` | absent the same way | `types-of-mappings/concept-type/manual.adoc`, §ConceptMaps | a non-empty string |
| `spec.conceptmap` | absent | the same section, which attaches a concept map "inside the header" | a non-empty string |
| `spec.openEhrConfig.revision` | absent from the closed `openEhrConfig`, so the key the prose writes cannot appear | `basics/main.adoc`, §Spec: "the `openEHRCondig.revision` states what revision of the archetype this mapping applies for", and the worked example writes it | a non-empty string |
| `spec.unidirectional` | absent | `basics/main.adoc`, §Direction, which pins a whole file to one direction | the same case-insensitive pattern the mapping-level key takes |
| `manual` | `type: array` carrying object `properties` and no `items`, so it validates nothing | `types-of-mappings/concept-type/manual.adoc` | items typed: `name` required, `fhir` and `openehr` each one `{path, value}` pair or a list of them, plus the two conditions, `value` and `unidirectional` |
| `hierarchy.split.openehr` | written outside `properties`, so it is unvalidated | `types-of-mappings/concept-type/HierarchyMappings.adoc` | typed exactly as `hierarchy.split.fhir` is |
| `operator` | a free string | the operator table in `basics/Conditions.adoc`, §operator | an enum of `one of`, `not of`, `empty`, `not empty`, `type` |
| `unidirectional` | a free string | `basics/main.adoc` §Direction and `basics/body.adoc` §Undirectional fix two values | a case-insensitive pattern over those two values |
| the data-type `type` | an enum at mapping level, where the prose never writes it, and a free string inside `with`, where every example does | `types-of-mappings/data-type/data-mappings.adoc` | the same enum in both places |
| `criteria` | a plain string, with `criterias` an untyped array | "`criteria` can be either a single element, or as a list" (`basics/Conditions.adoc`, §criteria) | a string or an array of strings |
| every object | open at the root, in `metadata`, in `spec` and in `preprocessor` | a closed grammar | `additionalProperties: false` at every level |

## What these keep from the published schemas

| Item | Published schema | Here |
|---|---|---|
| `openehrCondition` attributes | `oneOf` requiring `targetAttribute` or `targetAttributes`, on the preprocessor-level and mapping-level key alone | the same requirement, guarded by `if: {type: object}` so an explicitly null condition stays admitted, as `type: ["object", "null"]` says it is. The `oneOf` on a null instance matches both branches and so refuses it, which the published schema's own `type` contradicts |
| `fhirCondition` attributes | no such requirement | no such requirement |
| a `manual` entry's conditions | `fhirCondition` typed `object` with nothing required | the shared condition definition, with no attribute requirement |

A condition is one object per key here, which is what the published schemas
type and what every file of the mapping library writes. `basics/Conditions.adoc`
§type says "there can be more conditions ... notice that the condition is an
array", which no schema and no published file agrees with; the contradiction is
recorded as an `upstream-report` issue on this repository's tracker.

## Two case rules, and why they differ

A JSON Schema `enum` matches case-sensitively, so every value the published
schemas fix with an `enum` stays case-exact here: `type`, `spec.system`,
`spec.version`, `extension`, and the data-type `type`. `unidirectional` is the
exception and uses a case-insensitive pattern, because the specification's own
text is case-inconsistent about its keyword values (`basics/Conditions.adoc`
writes `openEHRCondition` beside `openehrCondition`, and
`types-of-mappings/concept-type/Reference.adoc` writes `openEHR:` beside
`openehr:`), so no case-exact reading of a keyword value is available from the
source. The same rule holds for the `$name` path variables, which no schema
constrains and `fhirconnect::model::ast::Variable` compares
case-insensitively.

## What these deliberately do not check

Three rules a JSON Schema could express stay in
`fhirconnect::model::semantic`, so each rule has exactly one home and one
diagnostic: `criteria` present exactly when the operator takes one, a
condition's `targetRoot` against the `with` path it filters, and every
cross-file reference against the loaded set. `profile.url` and `template.id`
stay optional, as the published context schema leaves them.
