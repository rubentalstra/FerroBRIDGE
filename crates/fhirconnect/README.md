<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# fhirconnect

The FHIRconnect mapping language for Rust: the mapping file model and its validation, context resolution into an immutable program, and the bidirectional interpreter between openEHR compositions and FHIR resources.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

Version 0.0.0 reserves the crate name. The implementation lands with
[FerroBRIDGE issue #1](https://github.com/rubentalstra/FerroBRIDGE/issues/1),
and the design is recorded in the repository's architecture document.

## The tree module

`fhirconnect::tree` is the bidirectional path model behind `with.fhir`. A
mapping runs both ways through one expression, and two of the forms FHIRconnect
writes are not FHIRPath at all (`$fhirRoot` and the `^` parent operator), so the
module parses the expression itself, resolves it against the element table
`fhir-types` emits per FHIR version, and evaluates it over that crate's lexical
`Value` tree.

- **The element table is the authority.** Navigation is by element name, a
  choice element resolves through `ofType()`, `as()` or the suffixed name, a
  repeating element is an array indexed by a structured occurrence, and a
  primitive's `extension` lives in the sibling member named with a leading
  underscore ([FHIR R4 JSON](https://hl7.org/fhir/R4/json.html)).
- **Every expression is classified when it is parsed.** `where()`, `first()`,
  `last()`, an index filter and `resolve()` select among the values a document
  already holds, so an expression carrying one is read-only and a write through
  it is refused with the offending step named.
- **`resolve()` fetches nothing.** It returns a deferred outcome carrying the
  reference and the steps still to apply, which the engine finishes.
- **A write is all or nothing.** It applies to a copy and replaces the document
  only when every step succeeded.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
