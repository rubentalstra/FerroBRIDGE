<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# openehr-mapping-core

The shared foundation of the FerroBRIDGE mapping languages: the common header
model, the YAML loader, the mapping registry, the diagnostic model, and openEHR
path resolution and execution over the Web Template.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

Version 0.0.0 reserves the crate name. The implementation lands with
[FerroBRIDGE issue #74](https://github.com/rubentalstra/FerroBRIDGE/issues/74),
and the design is recorded in the repository's architecture document.

openEHR is a registered trademark of the openEHR Foundation. This crate is not
endorsed by the openEHR Foundation.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
