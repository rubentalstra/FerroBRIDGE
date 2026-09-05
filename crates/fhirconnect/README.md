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

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
