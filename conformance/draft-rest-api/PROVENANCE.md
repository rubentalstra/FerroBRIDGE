<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the draft REST API pass list

`pass-list.txt` records which FSH operation definitions of the draft
FHIRconnect REST API chapter pass, one file name per line, and the definition
count on its `total` line. It holds no copy of the chapter. The definitions are
`docs/specs/fhirconnect/draft-rest-api/rest/input/fsh/operations/`, at the pin
`docs/VERSIONS.md` records and `docs/specs/fhirconnect/PROVENANCE.md`
describes.

- Written by:
  `operations::definitions::conformance_the_draft_operation_definitions_hold_their_pass_list`
  in `crates/fhirconnect/tests/it/operations/definitions.rs`, under
  `scripts/checks/conformance.sh --update`.
- A definition passes when the wire contract reads every `in` parameter and
  part it declares, and the operation answers only the `out` parameters it
  declares, with each `min = 1` one present.
- Never edit the list by hand.
