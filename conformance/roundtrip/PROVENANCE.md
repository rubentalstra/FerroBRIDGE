<!-- SPDX-FileCopyrightText: Vernum Projecten B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Provenance: the FHIR round-trip pass list

`pass-list.txt` records which round-trip chains hold both lens laws, one chain
name per line, and the chain count on its `total` line. It holds no copy of any
input. The chains read:

- `ferrobridge_diagnose_minimal`: the synthetic context and model under
  `crates/fhirconnect/tests/fixtures/`, with the synthetic diagnosis template
  and Condition of `tools/ferrobridge-testkit/fixtures/`.
- `kds_diagnose`: the published KDS diagnosis files of
  `docs/specs/fhirconnect-mapping-lib/`, the project directory
  `crates/fhirconnect/tests/fixtures/projects/ferrobridge/kds_diagnose/`, and
  the published `KDS_Diagnose` template under
  `tools/ferrobridge-testkit/fixtures/opt/kds/`, which carries its own
  `PROVENANCE.md`.

The list is written by
`roundtrip::conformance_the_round_trip_chains_hold_their_pass_list` in
`crates/fhirconnect/tests/it/roundtrip.rs`, under
`scripts/checks/conformance.sh --update`. A chain passes when `PutGet` and
`GetPut` both run and each declares exactly the set its reviewed snapshot pins.
Never edit the list by hand.
