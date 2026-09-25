<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# omocl

The OMOCL mapping language for Rust: the mapping file model and its validation, and the interpreter emitting OMOP CDM rows from openEHR compositions.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

Version 0.0.0 holds the crate name on crates.io until the first publish. The
crate is built module by module under
[FerroBRIDGE issue #2](https://github.com/rubentalstra/FerroBRIDGE/issues/2),
and the design is recorded in the repository's architecture document.

## The model module

`omocl::model` is the OMOCL file model and the layers that validate it. OMOCL
publishes its grammar as two railroad images and four syntax tables and no
schema, so the module reads those artefacts first and the OMOCL mapping library
second.

- **`ast`** carries one positioned Rust type per construct: the shared header,
  `spec.system` and `spec.version` (OMOP CDM 5.4), the twelve entry `type`
  values (ten CDM targets, `Include` and `CustomMapping`), column entries with
  `optional` and ordered `alternatives`, each exactly one of `path`, `code`,
  `conceptMap` and `multiplication`, and `base_path`.
- **`parse`** lowers the positioned YAML tree the shared loader produces. It
  refuses a key the grammar does not define with the file, line, column and key
  in the diagnostic, and collects every refusal of a file.
- **`schema`** validates the same document against
  `schemas/omocl-mapping.schema.json`, the JSON Schema FerroBRIDGE authors for
  OMOCL, closed at every level.
- **`projection`** is the table from an OMOCL key to the CDM v5.4 columns it
  writes, for all ten targets: `concept_id` writes the standard concept, the
  source value and the source concept; `value`, `unit`, `range_low`,
  `range_high` and `operator_concept_id` read one `DV_QUANTITY` node; a date
  key writes the `_date` and `_datetime` columns; `procedure_start_date` writes
  the CDM's `procedure_date`. Every column it names is checked against the
  generated CDM metadata of `omop-cdm`.
- **`semantic`** holds the rules a schema cannot express: two keys of one
  record writing one column, a `CustomMapping` naming no registered converter,
  an `Include` naming an archetype no loaded file maps, and, once a vocabulary
  is loaded, a literal concept id outside the domain its column takes.
- **`load`** runs all of it over one file or a set, indexed by mapping name and
  archetype through the shared registry.

199 of the 208 files of the vendored library load; the nine it refuses are
pinned by the tests with the defect each carries.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
