<!-- SPDX-FileCopyrightText: Ruben Talstra -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# openehr-mapping-core

The shared foundation of the FerroBRIDGE mapping languages. FHIRconnect and
OMOCL are two languages over one header, so this crate models the header once
and both language crates build on it:

- **The header:** `grammar` (the language and its version), `type`,
  `metadata.name`, `metadata.version`, and `spec` with
  `spec.openEhrConfig.archetype`. Each identifier is its own type, and
  everything a language adds under `spec` stays an opaque positioned node.
- **The YAML loader:** one file into a header plus a value tree where every
  node carries its line and column, with anchors, aliases and merge keys
  resolved. A duplicate key, an alias with no anchor, and a tab used for
  indentation are refusals, never silent repairs.
- **The registry:** loaded files indexed by mapping name and by archetype id. A
  second file declaring a name already loaded is refused naming both files.
- **The diagnostic:** file, position, mapping name, model path, severity, code
  and message, rendered as `file:line:col: severity[code]: message (name)`.
- **The path model:** an openEHR RM path parsed by `openehr-rm`, plus the `../`
  parent step and the `$name` variables the mapping languages add. Resolution
  against an anchor refuses a step above its root, and a resolved path carries
  no `..`.

Part of [FerroBRIDGE](https://ferrobridge.eu), a pure-Rust bridge between
openEHR and two interoperability targets: HL7 FHIR through the FHIRconnect
specification, and the OMOP Common Data Model through the OMOCL specification.

Version 0.0.0 reserves the crate name on crates.io.

openEHR is a registered trademark of the openEHR Foundation. This crate is not
endorsed by the openEHR Foundation.

## Licence

Business Source License 1.1 (`LICENSE`): free for every non-production use and
for non-commercial production use; a commercial licence for other production
use; Apache License 2.0 four years after each version.
